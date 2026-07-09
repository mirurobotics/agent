# ExecPlan: S3 multipart — make mid-part local read errors terminal (`LocalIoErr`)

- **Date:** 2026-07-09
- **Repo:** `/home/ben/miru/workbench6/repos/agent` (Rust, crate `miru-agent`)
- **Branch:** `feat/s3-multipart-upload` (S3 multipart PR #126)
- **Delivery:** push-mode to PR #126 (do NOT open a new PR)
- **Status:** backlog (research complete; not implemented)

> Note on repo layout: the crate lives in a nested `agent/` directory. All source
> paths below are `agent/src/...` and `agent/tests/...` under the repo root. The
> `scripts/` and `plans/` directories are at the repo root.

## Problem

In `agent/src/s3/multipart.rs`, `Store::upload_part` builds the part body lazily:

```rust
let body = ByteStream::read_from()
    .path(src.path())
    .offset(offset)
    .length(Length::Exact(length))
    .build()
    .await
    .map_err(|e| errors::map_bytestream_err("upload_part", dst, src, &e))?;
```

`.build().await` only *opens* the file; the part's bytes are read lazily by the
SDK during `.send()`. A mid-part disk read error (source shrunk / truncated /
disk fault after open) therefore surfaces inside `send()` as
`SdkError::DispatchFailure`, which `errors::map_sdk_err` classifies as a
**retryable** `ConnectionErr`. This is inconsistent with the `get()` path in
`agent/src/s3/mod.rs`, where a local I/O failure is a **terminal** `LocalIoErr`
(via `errors::map_body_io_err`). A local disk fault is not a transient network
condition and must not be retried.

The current source even documents this as a known limitation (the doc comment on
`upload_part`, lines ~148–157). This plan removes that limitation.

## Decided approach (bounded per-part buffering)

Before sending each part, read its byte range `[offset, offset + length)` from
the source file into an in-memory `Vec<u8>`, mapping any open/seek/read error to
terminal `LocalIoErr`. Then send the buffered bytes via `ByteStream::from(buf)`.

This bounds peak memory to **one part** (`part_size_for(size)` — 8 MiB for files
up to `8 MiB × 10_000`, growing only for very large files), not the whole file.
The part-by-part streaming model is preserved; only the per-part body source
changes from a lazy file handle to a pre-read buffer.

### Resolved concrete decisions (verified against the code)

1. **No range-read helper exists in `agent/src/filesys`.** `files.rs` has
   `read_bytes` (whole file), `hash` (streamed 8 KiB), `read_secret_bytes`,
   `size`, etc., but nothing that seeks to an offset and reads a bounded range.
   `file.rs` exposes only `path()` (via `PathExt`), `name()`, `parent()`. There
   is no seek/`read_at`/`read_range` primitive, and no existing `SeekFrom` /
   `AsyncSeek` usage anywhere in `agent/src`.
   → **Use the tokio::fs fallback**, inline in `multipart.rs` (or as a small
   private helper in `multipart.rs`). Do not add a new public `filesys` API for
   this one caller.

2. **Error mapper: reuse `errors::map_body_io_err`.** Signature (confirmed,
   `agent/src/s3/errors.rs:194`):
   ```rust
   pub fn map_body_io_err(operation: &str, obj: &Object, file: &File, err: std::io::Error) -> S3Err
   ```
   It takes an owned `std::io::Error` and produces `S3Err::LocalIoErr`. Our
   seek/read produces exactly a `std::io::Error`, so this fits without a new
   mapper. (`map_bytestream_err` takes a `&ByteStreamError` — no longer relevant
   once we stop calling `ByteStream::read_from().build()`.) Keep all mappers in
   `errors.rs`; **do not add a new mapper** unless the message wording needs to
   differ — `map_body_io_err`'s message (`filesystem I/O error at path '{file}':
   {err}`) is appropriate for open/seek/read failures.

3. **`ByteStream::from` input type: `Vec<u8>`.** aws-sdk-s3 is `1.137`
   (workspace `Cargo.toml:26`); `aws_sdk_s3::primitives::ByteStream` implements
   `From<Vec<u8>>` (and `From<Bytes>` / `From<&'static [u8]>`). The `bytes` crate
   is **not** a direct dependency and there is no reason to add it — the buffer
   is a runtime-built `Vec<u8>`, so `ByteStream::from(buf)` is the direct,
   dependency-free choice.

4. **Types:** `offset: u64`, `length: u64` (current `upload_part` signature).
   The buffer is allocated as `vec![0u8; length as usize]`; `read_exact` fills it.
   `length` per part is `≤ part_size_for(size)`, which is bounded by S3's part
   rules, so the `as usize` cast is safe on 64-bit targets (the agent's targets).

### Feasibility caveats

- **Peak memory rises from ~O(64 KiB SDK chunk) to O(part_size).** For the
  common case that is 8 MiB held per in-flight part; parts are uploaded
  sequentially (`upload_parts` loops one at a time), so it is one part at a time,
  not all parts. This is the intended, documented trade-off. For pathologically
  large files `part_size_for` grows the part (to keep ≤ 10,000 parts), so the
  buffer grows correspondingly — still exactly one part.
- **`read_exact` semantics:** if the file shrank below `offset + length`,
  `read_exact` returns `io::ErrorKind::UnexpectedEof`, which maps to
  `LocalIoErr` — exactly the terminal classification we want. This makes the
  shrink/TOCTOU case deterministic (previously it depended on when the SDK read
  hit EOF mid-dispatch).
- **Correctness of the send:** `ByteStream::from(Vec<u8>)` sets a known content
  length equal to the buffer size, matching the `part_plan` length. No change to
  the completed-part / ETag handling.

## Milestones

### M1 — Buffer the part body and reclassify local read errors as terminal

**File:** `agent/src/s3/multipart.rs`

Steps:

1. Update imports:
   - Remove `use aws_sdk_s3::primitives::Length;` (no longer used) if `Length`
     is otherwise unused in the file. Keep `ByteStream`.
   - Add `use tokio::io::{AsyncReadExt, AsyncSeekExt};` and
     `use std::io::SeekFrom;` (grouped per the repo's import conventions:
     standard crates, then internal `crate::` absolute imports, then external
     crates — mirror the existing grouping/comment style in the file).
   - `errors` is already imported (`use crate::s3::errors;`).

2. Add a small private async helper on `impl Store` (or a free `async fn` in the
   module) that reads a bounded range into a buffer, mapping every io error to
   `LocalIoErr` via `map_body_io_err`. Suggested shape:

   ```rust
   /// Reads `src[offset..offset+length]` into an in-memory buffer, mapping any
   /// open/seek/read failure to a terminal [`S3Err::LocalIoErr`]. Peak memory is
   /// one part (`length` bytes); the caller uploads parts sequentially, so at
   /// most one part is buffered at a time.
   async fn read_part_bytes(
       src: &File,
       dst: &Object,
       offset: u64,
       length: u64,
   ) -> Result<Vec<u8>, S3Err> {
       let mut f = tokio::fs::File::open(src.path())
           .await
           .map_err(|e| errors::map_body_io_err("upload_part", dst, src, e))?;
       f.seek(SeekFrom::Start(offset))
           .await
           .map_err(|e| errors::map_body_io_err("upload_part", dst, src, e))?;
       let mut buf = vec![0u8; length as usize];
       f.read_exact(&mut buf)
           .await
           .map_err(|e| errors::map_body_io_err("upload_part", dst, src, e))?;
       Ok(buf)
   }
   ```
   (`Object` and `File` are both already in scope: `crate::s3::{Object, ...}` and
   `crate::filesys::file::File`.)

3. Rewrite the body-building part of `upload_part` to buffer then send:

   ```rust
   let buf = Self::read_part_bytes(src, dst, offset, length).await?;
   let body = ByteStream::from(buf);

   let output = self
       .client
       .upload_part()
       // ... unchanged ...
       .body(body)
       .send()
       .await
       .map_err(|e| errors::map_sdk_err("upload_part", Some(dst.key.to_string()), e))?;
   ```
   The `upload_part` signature (`src, dst, upload_id, part_number, offset,
   length`) is unchanged, as is the ETag extraction and `CompletedPart` build.

4. **Doc comments (required):**
   - Rewrite the `upload_part` doc comment (currently lines ~148–157). Remove the
     paragraph stating that a mid-stream read surfaces as a retryable
     `ConnectionErr` ("known classification limitation"). Replace with: the part
     range is read into a bounded in-memory buffer before sending; any local
     open/seek/read failure is terminal `LocalIoErr` (consistent with the
     `get()` path); peak memory is one part.
   - Check the module-level doc (top of `multipart.rs`) and `put_multipart` /
     `upload_parts` docs for any "streamed / never held in memory" phrasing that
     is now inaccurate for the per-part window, and adjust to "streamed
     part-by-part with a bounded per-part buffer (peak = one part)."
   - Check `Store::put` / `put_singlepart` docs in `agent/src/s3/mod.rs`
     (lines ~106–120) — the "the whole file is never held in memory" claim is
     still true (peak = one part, not the whole file), but confirm the wording
     doesn't imply constant/`get()`-style streaming for the multipart path.

**Do NOT touch** the resume path (there is no resume logic in this stateless
module — confirm nothing else references `ByteStream::read_from` before removing
`Length`).

### M2 — Tests (offline `StaticReplayClient` harness)

**File:** `agent/tests/s3/multipart.rs`

1. **Existing `source_missing` tests must still pass, unchanged.** Both
   `put_multipart_missing_source_maps_to_local_io_err` (nonexistent path) and
   `put_multipart_deleted_source_maps_to_local_io_err` (deleted after sizing)
   already assert `S3Err::LocalIoErr(_)` and the `[create, abort]` shape. With
   the new synchronous read they fail at `tokio::fs::File::open` inside
   `read_part_bytes` (terminal `LocalIoErr`) instead of at the old
   `ByteStream::read_from().build()`. Same variant, same wire shape — they should
   pass as-is. Verify the replay fixtures still expect exactly `[create_shape(),
   abort_shape()]` (they do: `upload_part` never leaves the client because the
   local open fails first).

2. **ADD a shrink/TOCTOU test** (in `pub mod source_missing`, or a sibling
   `truncated_source` module):
   - Seed a temp file large enough that its recorded `size` exceeds its later
     on-disk length (e.g. write N bytes, build the `Source` via `source_of`
     capturing `size = N`, then truncate the file to `< N` — or construct a
     `Source` with a `size` larger than the bytes actually written, matching the
     existing "claimed size > 0" pattern used by the missing-source test).
   - **Truncation mechanism:** there is no filesys truncate helper. Use the std/
     tokio API directly, e.g.
     `tokio::fs::OpenOptions::new().write(true).open(path).await?.set_len(k).await?`
     (or `std::fs::File::open(...).set_len(k)`), truncating to a length shorter
     than the recorded `size`. Keep the temp file alive via its `TempFile` guard.
   - Replay events: `[create_resp, abort_resp]` (same as the missing-source
     tests) — `read_exact` hits `UnexpectedEof` before any `upload_part` request
     leaves the client.
   - Assert `matches!(err, S3Err::LocalIoErr(_))` and
     `actual_shapes(&replay) == vec![create_shape(), abort_shape()]`.
   - Comment that this is now **deterministic**: the old lazy-read path could
     surface EOF as a retryable `ConnectionErr` depending on SDK dispatch timing;
     the pre-read makes it a terminal `LocalIoErr` every time.

3. **Part-body correctness:** already covered by the `part_plan` unit tests in
   `multipart.rs` (offsets/lengths/coverage) plus the shape assertions in the
   existing `put` tests (`small_file_uploads_as_single_part`, etc.), which
   continue to drive create → upload_part → complete. Adding a byte-level
   assertion on the sent part body is **optional** — only add it if the
   `StaticReplayClient` fixtures make the recorded request body easy to read
   back (the current fixtures use `shape(...)` which deliberately ignores
   bodies). If not trivially readable, skip it; the range math is already
   unit-tested.

## Validation (required before publishing to PR #126)

1. `scripts/preflight.sh` must report **`clean`** (zero-warning build + covgate +
   lint). Run from the repo root. Do not push until it is clean.
2. Import linter in CHECK mode must exit `0`: absolute `crate::` imports, grouped
   same-crate imports (mirror the existing import blocks in `multipart.rs`).
3. Targeted test run: the `s3::multipart` offline suite passes, including the two
   pre-existing `source_missing` tests and the new shrink/TOCTOU test, and the
   `errors.rs` unit tests are unaffected.
4. Confirm `Length` import removal did not break any remaining use, and that
   `bytes` was NOT added as a dependency (`ByteStream::from(Vec<u8>)` needs none).

## Constraints (record and honor)

- **SSH-signed commits** — never disable `commit.gpgsign`; mirurobotics requires
  verified signatures.
- **`scripts/preflight.sh` clean** — build zero-warnings + covgate + lint — is
  the gate before any push.
- **Import linter CHECK mode exit 0** — absolute `crate::` imports, grouped
  same-crate imports.
- **Thin client** — no new parsing/validation surface; this is a local I/O
  classification fix only.
- **Do NOT touch the resume path.**
- **Push-mode delivery to PR #126** — commit onto `feat/s3-multipart-upload` and
  push; do not open a new PR.

## Out of scope

- Adding a general-purpose range-read primitive to `agent/src/filesys` (only one
  caller; keep the read inline/private in `multipart.rs`).
- Parallel part uploads or any change to `upload_parts` sequencing.
- Adding the `bytes` crate.
