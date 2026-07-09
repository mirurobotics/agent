# ExecPlan: S3 multipart upload module for miru-agent (PR 2/4)

- **Repo:** `/home/ben/miru/workbench6/repos/agent` (Rust crate `miru-agent`, nested under `agent/`)
- **Branch:** `feat/s3-multipart-upload` off `main` (`01cbffd` — merged S3 CRUD client, PR 1/4)
- **Scope:** One focused PR (2/4 of the split of monolithic PR #102). Adds streaming multipart upload + size-based routing. **No durable resume state** (that is a later PR — see "Out of scope").
- **Date:** 2026-07-09

## Context / Goal

`main` currently ships a thin, stateless `s3::Store` CRUD client
(`agent/src/s3/mod.rs`). `Store::put` streams the whole file through a single
`PutObject` and its doc comment explicitly promises size-based routing to a
multipart upload "with the multipart module in a follow-up PR." This PR delivers
that follow-up.

Goal: add `agent/src/s3/multipart.rs` implementing a **stateless one-shot**
multipart upload (`create_multipart_upload` → stream file off disk part-by-part
via `upload_part` → `complete_multipart_upload`; abort on any failure), wire
size-based routing into `Store::put`, give `S3Err::InvalidResponseErr` its first
real producers, and cover it all with offline `StaticReplayClient` tests. The
`Store` stays a thin client: caller-supplied temp credentials only, no ambient
AWS config, and the file is streamed from disk (never buffered whole).

### Source of truth

The implementation is carved from PR #102
(`origin/feat/s3-object-storage-crud:agent/src/s3/multipart.rs` and its test file
`agent/tests/s3/multipart.rs`). **That code targets an older version of the s3
module** with a different error taxonomy and test harness; the "Key adaptations"
section below enumerates exactly what must change for `main`'s API. Read #102's
files with `git show` before implementing, but treat this plan's adapted
signatures as authoritative.

## Key adaptations from PR #102 → current `main`

| #102 (old) | `main` (current) | Action |
|---|---|---|
| `errors::map_sdk_err_common(op, key, e)` | `errors::map_sdk_err(op, key, e)` | **Rename** every call site in the carved code to `map_sdk_err`. |
| `self.map_bytestream_err("upload_part", dst, src, &e)` (a `self`-method on `Store`) | `errors::map_bytestream_err(op, obj, file, &e)` (free fn in `errors.rs`) | Call the **free function**, not a method. Signature is `(operation, &Object, &File, &ByteStreamError)`. |
| `use super::errors::{self, InvalidResponseErr, NoSuchUploadErr};` | — | Drop `NoSuchUploadErr` (does not exist in `main`; it belonged to the resume path). Import `InvalidResponseErr` from `crate::s3::errors`. |
| `use super::{Object, S3Err, Store, PART_SIZE};` | — | Convert to **absolute** `crate::s3::{...}` imports (the import linter normalizes `super::` to `crate::`; use `crate::` directly). |
| `resume_multipart_upload`, `list_parts`, `list_parts_page`, `NoSuchUploadErr`, `HashMap`, `ListPartsOutput`, `Length` (for resume) | — | **Omit entirely** — resume is out of scope for PR 2/4. Keep only the create → upload_parts → complete → abort path plus `part_size_for`. |
| `PART_SIZE` = `8 * 1024 * 1024` (8 MiB), defined in `mod.rs` | not yet defined | **Add** `const PART_SIZE: u64 = 8 * 1024 * 1024;` to `mod.rs` (same value #102 used). |
| `put` routing `if size > PART_SIZE { put_multipart } else { put_singlepart }` | `put` calls `put_singlepart` only | **Add** the routing (keep `put_singlepart` public, unchanged). |
| Test harness: `NamedTempFile` from `tempfile`, `temp_file_with(&[u8]) -> NamedTempFile`, `std::fs::metadata(...).len()` | `files::TempFile`, `temp_file_with(&[u8]) -> files::TempFile` (async), `files::size(&File)` | **Rewrite** the test fixtures to use `main`'s harness helpers (`agent/tests/s3/mod.rs`). See Tests section. |

### What does NOT change

- `part_size_for` math and its three unit tests port over verbatim (they only
  reference `PART_SIZE`, `MIN_PART_SIZE`, `MAX_PARTS`).
- The `PartToUpload` / `UploadedPart` / `Source` structs.
- The abort-on-failure control flow (`put_multipart` splits create from
  `exec_multipart_upload` so a single `?` funnels through one abort site).
- `upload_part` and `create_multipart_upload` are the two `InvalidResponseErr`
  producers (see below).

### `InvalidResponseErr` producers (first real users of this variant)

`S3Err::InvalidResponseErr` is currently UNREACHABLE in `main` (reserved for this
PR). It gets **two** producers in `multipart.rs`:

1. **`create_multipart_upload`** — when the `CreateMultipartUpload` response omits
   an `upload_id`:
   ```rust
   .ok_or_else(|| S3Err::InvalidResponseErr(InvalidResponseErr {
       operation: "create_multipart_upload".to_string(),
       msg: "response did not include an upload id".to_string(),
       trace: trace!(),
   }))?
   ```
2. **`upload_part`** — when the `UploadPart` response omits the part `ETag`:
   ```rust
   output.e_tag().map(str::to_string).ok_or_else(|| {
       S3Err::InvalidResponseErr(InvalidResponseErr {
           operation: "upload_part".to_string(),
           msg: "response did not include an etag".to_string(),
           trace: trace!(),
       })
   })
   ```

## Out of scope (later PRs)

- Durable resume state / `resume_multipart_upload` / `list_parts` / `NoSuchUploadErr`
  / the `s3::uploader` layer (PR 3/4+, per
  `origin/feat/s3-object-storage-crud:plans/completed/20260704-s3-thin-client-uploader-split.md`).
- Any production caller of `Store` (none exists yet).

---

## Milestone 1 — Add `PART_SIZE` + size routing in `mod.rs`

Small, self-contained, compiles and passes existing tests before `multipart.rs`
exists (routing calls `self.put_multipart`, so this milestone lands together with
M2 in one commit; keep them ordered for review clarity).

**File: `agent/src/s3/mod.rs`**

1. Add the module declaration near the existing `pub mod errors;`:
   ```rust
   pub mod errors;
   pub mod multipart;
   ```
2. Add the part-size constant (module-level, after the `use` block):
   ```rust
   /// Objects larger than this stream through a multipart upload; objects at or
   /// below it go through a single `PutObject`. S3's own multipart part-size
   /// floor is 5 MiB; 8 MiB gives headroom while keeping part counts small.
   const PART_SIZE: u64 = 8 * 1024 * 1024; // 8 MiB
   ```
3. Re-export the public multipart types (so tests and future callers reach them):
   ```rust
   pub use multipart::{PartToUpload, Source, UploadedPart};
   ```
4. Replace the body of `put` (currently delegates unconditionally to
   `put_singlepart`) with size-based routing, and update its doc comment (drop
   the "arrives in a follow-up PR" language):
   ```rust
   /// Creates or overwrites an object by streaming a file off disk.
   ///
   /// The whole file is never held in memory: files at or below [`PART_SIZE`]
   /// stream through one `PutObject` ([`Self::put_singlepart`]); larger files
   /// stream part-by-part through a stateless multipart upload
   /// ([`Self::put_multipart`]).
   pub async fn put(&self, src: File, dst: &Object) -> Result<(), S3Err> {
       let size = crate::filesys::files::size(&src).await?;
       if size > PART_SIZE {
           self.put_multipart(&multipart::Source { file: src, size }, dst)
               .await
       } else {
           self.put_singlepart(&src, dst).await
       }
   }
   ```
   - `files::size` returns `Result<u64, FileSysErr>`; `?` converts via the
     existing `From<filesys::FileSysErr> for S3Err`. Confirmed present at
     `agent/src/filesys/files.rs:521`.
   - Keep `put_singlepart` exactly as-is (still `pub`, still used by routing and
     by existing tests).
   - Add `use crate::filesys::files;` to the internal-crates import group if not
     already importing `files` (currently `mod.rs` imports
     `crate::filesys::{file::File, path::PathExt}`; extend to include `files`, or
     reference `crate::filesys::files::size` inline as shown).

**Verify:** `cargo build -p miru-agent --features test` compiles once M2 lands.

## Milestone 2 — Create `agent/src/s3/multipart.rs`

Carve the stateless multipart path from #102, adapted per the table above.

**New file: `agent/src/s3/multipart.rs`**

Imports (absolute `crate::` paths, grouped per AGENTS.md — no `super::`):
```rust
//! Multipart upload machinery for [`Store`].
//!
//! S3 requires objects larger than a single `PutObject` to be uploaded in
//! parts: create an upload, upload each chunk, then complete (or abort) it. This
//! module holds the stateless multipart surface ([`Store::put_multipart`]) over a
//! set of internal per-part primitives (create / upload_part / complete / abort)
//! plus the part-sizing policy ([`Store::part_size_for`]). The single-part path
//! and the rest of the object API live in the parent [`crate::s3`] module.

// internal crates
use crate::filesys::file::File;
use crate::filesys::path::PathExt;
use crate::s3::errors::{self, InvalidResponseErr};
use crate::s3::{Object, S3Err, Store, PART_SIZE};
use crate::trace;

// external crates
use aws_sdk_s3::primitives::{ByteStream, Length};
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart};
```
> Note: `PART_SIZE` is a private (`const`, module-private) item of `crate::s3`.
> A sibling submodule can import it via `crate::s3::PART_SIZE` because
> module-privacy makes it visible to descendants of the defining module. If the
> import linter or visibility complains, mark it `pub(crate) const PART_SIZE` in
> `mod.rs` (preferred — explicit and lint-clean).

Type aliases and constants:
```rust
type UploadID = String;
type ETag = String;

// S3-defined hard limits.
const MIN_PART_SIZE: u64 = 5 * 1024 * 1024; // 5 MiB
const MAX_PARTS: u64 = 10_000; // 10,000 parts
```

Public data types (ported verbatim from #102):
```rust
#[derive(Debug, Clone)]
pub struct PartToUpload {
    pub upload_id: String,
    pub number: i32,
    pub offset: u64,
    pub length: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadedPart {
    pub number: i32,
    pub etag: String,
}

pub struct Source {
    pub file: File,
    pub size: u64,
}
```

`impl Store` methods (adapted — omit all resume machinery):

1. **`put_multipart`** (public, stateless one-shot; create → exec → abort-on-error):
   ```rust
   pub async fn put_multipart(&self, src: &Source, dst: &Object) -> Result<(), S3Err> {
       let upload_id = self.create_multipart_upload(dst).await?;
       match self.exec_multipart_upload(src, dst, &upload_id).await {
           Ok(()) => Ok(()),
           Err(err) => {
               // Best-effort cleanup: don't mask the original error.
               let _ = self.abort_multipart_upload(dst, &upload_id).await;
               Err(err)
           }
       }
   }
   ```
2. **`part_size_for`** (`pub(crate)`, verbatim):
   ```rust
   pub(crate) fn part_size_for(size: u64) -> u64 {
       if size.div_ceil(PART_SIZE) <= MAX_PARTS {
           PART_SIZE
       } else {
           size.div_ceil(MAX_PARTS).max(MIN_PART_SIZE)
       }
   }
   ```
3. **`create_multipart_upload`** (public; **InvalidResponseErr producer #1**):
   ```rust
   pub async fn create_multipart_upload(&self, dst: &Object) -> Result<UploadID, S3Err> {
       let created = self.client
           .create_multipart_upload()
           .bucket(&dst.bucket).key(&dst.key)
           .send().await
           .map_err(|e| errors::map_sdk_err("create_multipart_upload", Some(dst.key.to_string()), e))?;
       let upload_id = created.upload_id()
           .ok_or_else(|| S3Err::InvalidResponseErr(InvalidResponseErr {
               operation: "create_multipart_upload".to_string(),
               msg: "response did not include an upload id".to_string(),
               trace: trace!(),
           }))?
           .to_string();
       Ok(upload_id)
   }
   ```
4. **`exec_multipart_upload`** (public; single `?`-funnel for the abort site):
   ```rust
   pub async fn exec_multipart_upload(
       &self, src: &Source, dst: &Object, upload_id: &str,
   ) -> Result<(), S3Err> {
       let parts = self.upload_parts(src, dst, upload_id).await?;
       self.complete_multipart_upload(dst, upload_id, &parts).await
   }
   ```
5. **`upload_parts`** (private; streams each `part_size_for` chunk from byte 0 in
   order, 1-based part numbers — verbatim from #102, no changes).
6. **`upload_part`** (private; **InvalidResponseErr producer #2**; adapt the
   bytestream + sdk mappers):
   ```rust
   async fn upload_part(&self, src: &File, dst: &Object, part: &PartToUpload) -> Result<ETag, S3Err> {
       let body = ByteStream::read_from()
           .path(src.path())
           .offset(part.offset)
           .length(Length::Exact(part.length))
           .build().await
           .map_err(|e| errors::map_bytestream_err("upload_part", dst, src, &e))?;
       let output = self.client
           .upload_part()
           .bucket(&dst.bucket).key(&dst.key)
           .upload_id(&part.upload_id).part_number(part.number)
           .body(body)
           .send().await
           .map_err(|e| errors::map_sdk_err("upload_part", Some(dst.key.to_string()), e))?;
       output.e_tag().map(str::to_string).ok_or_else(|| {
           S3Err::InvalidResponseErr(InvalidResponseErr {
               operation: "upload_part".to_string(),
               msg: "response did not include an etag".to_string(),
               trace: trace!(),
           })
       })
   }
   ```
   - **Adaptation:** `map_bytestream_err` is a **free fn** in `main`
     (`errors::map_bytestream_err(operation, &Object, &File, &ByteStreamError)`),
     not the `self`-method #102 used. `map_sdk_err` replaces `map_sdk_err_common`.
7. **`complete_multipart_upload`** (private; builds `CompletedPart`/
   `CompletedMultipartUpload` internally — verbatim, but `map_sdk_err`).
8. **`abort_multipart_upload`** (public; verbatim, but `map_sdk_err`).

**Unit tests** (in `#[cfg(test)] mod tests` at the bottom of `multipart.rs`) —
port the three `part_size_for` tests verbatim (they only touch pure math):
`part_size_uses_fixed_size_below_the_part_ceiling`,
`part_size_grows_to_stay_under_the_part_ceiling`,
`part_size_never_drops_below_the_minimum`.

**Verify:** `cargo build -p miru-agent --features test` && `cargo test -p miru-agent --features test s3::multipart` compile and pass.

## Milestone 3 — Offline integration tests `agent/tests/s3/multipart.rs`

Use the **same `StaticReplayClient` harness** as `agent/tests/s3/mod.rs`. Register
the module the same way the existing s3 test tree is registered.

**File: `agent/tests/s3/mod.rs`** — add the submodule declaration alongside the
existing `pub mod put; pub mod get; ...` blocks:
```rust
pub mod multipart;
```
(Place it after `pub mod construction { ... }` and the shared helpers, next to the
other `pub mod` operation blocks. The helpers `uri`, `obj`, `req`, `resp`,
`resp_xml`, `store_with`, `store_expecting`, `temp_file_with`, and the constants
`REGION`, `BUCKET`, `IGNORED_HEADERS` remain in `mod.rs` and are reached via
`use super::*;`.)

**New file: `agent/tests/s3/multipart.rs`** — carved from #102's test file, **but
the `put`-path tests only** (drop the entire `resume` module). Adapt the fixtures
to `main`'s harness:

- `use super::*;` then `use miru_agent::s3::Source;` (re-exported from
  `crate::s3` in M1). #102 used `miru_agent::s3::multipart::Source` — either works;
  prefer `miru_agent::s3::Source` to match the re-export.
- **Fixture adaptation (critical):** `main`'s `temp_file_with(&[u8])` is `async`
  and returns `files::TempFile` (not `tempfile::NamedTempFile`). Rewrite
  `source_of` and any file setup accordingly:
  ```rust
  const UPLOAD_ID: &str = "test-upload-id";

  /// Builds a `Source` from a temp file, reading its length off disk with the
  /// crate's own `files::size`.
  async fn source_of(tf: &files::TempFile) -> Source {
      let file = tf.to_file();
      let size = files::size(&file).await.unwrap();
      Source { file, size }
  }
  ```
  `files` is already imported in `mod.rs` (`use miru_agent::filesys::{files, WriteOptions};`)
  and reachable via `use super::*;`. Add `use miru_agent::filesys::file::File;` if a
  test constructs a raw `File` (already imported in `mod.rs`).

### Canned multipart responses (how to build them with `StaticReplayClient`)

Each phase is one `ReplayEvent::new(request, response)`. Requests match on method +
path-style URI; POST bodies vary and are asserted by hand (see existing
`put_streams_file_body_bytes`). Helpers to define at the top of the `put` test
module (ported from #102):

- **create response** — 200 with `InitiateMultipartUploadResult` XML carrying
  `<UploadId>{UPLOAD_ID}</UploadId>`:
  ```rust
  fn create_resp() -> http::Response<SdkBody> {
      let xml = format!(
          r#"<?xml version="1.0" encoding="UTF-8"?>
  <InitiateMultipartUploadResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Bucket>{BUCKET}</Bucket><Key>big.bin</Key><UploadId>{UPLOAD_ID}</UploadId></InitiateMultipartUploadResult>"#);
      http::Response::builder().status(200)
          .header("content-type", "application/xml")
          .body(SdkBody::from(xml)).unwrap()
  }
  ```
- **upload_part response** — 200 with an `ETag` header:
  ```rust
  fn upload_part_resp(etag: &str) -> http::Response<SdkBody> {
      http::Response::builder().status(200)
          .header("ETag", etag)
          .body(SdkBody::empty()).unwrap()
  }
  ```
- **complete response** — 200 with `CompleteMultipartUploadResult` XML.
- Request builders use the operation query params:
  - create: `POST uri("big.bin?uploads&x-id=CreateMultipartUpload")`
  - part: `PUT uri("big.bin?x-id=UploadPart&partNumber=1&uploadId={UPLOAD_ID}")`
  - complete: `POST uri("big.bin?x-id=CompleteMultipartUpload&uploadId={UPLOAD_ID}")`
  - abort: `DELETE uri("big.bin?x-id=AbortMultipartUpload&uploadId={UPLOAD_ID}")`

### Tests to add (each asserts, in `pub mod multipart { pub mod put { ... } }`)

1. **`small_file_uploads_as_single_part`** (happy path, create → part → complete).
   A tiny body (`b"multipart-body"`) rides the multipart path because the 8 MiB
   part size dwarfs it → exactly one part. Wire 3 `ReplayEvent`s (create_resp,
   upload_part_resp, complete_resp). Call `store.put_multipart(&source_of(&src).await, &obj("big.bin"))`.
   **Asserts:** `.unwrap()` succeeds; `replay.actual_requests()` has len 3;
   request[0] is `POST` containing `uploads`; request[1] is `PUT` containing
   `partNumber=1` and `uploadId={UPLOAD_ID}`; request[2] is `POST` containing
   `uploadId={UPLOAD_ID}`.

2. **`create_without_upload_id_maps_to_invalid_response`** (InvalidResponseErr,
   producer #1). One `ReplayEvent`: create request → 200 with well-formed
   `InitiateMultipartUploadResult` XML that **omits `<UploadId>`**.
   **Asserts:** `matches!(err, S3Err::InvalidResponseErr(_))`.

3. **`part_without_etag_maps_to_invalid_response`** (InvalidResponseErr,
   producer #2 — **new test, not in #102**; needed because #102 had no direct
   test for the `upload_part` missing-ETag branch). Wire create_resp then a
   `upload_part` 200 response with **no `ETag` header** (`SdkBody::empty()`), then
   an abort request/resp (the missing ETag makes `upload_part` return
   `InvalidResponseErr`, which propagates through `exec_multipart_upload` and
   triggers the best-effort abort).
   **Asserts:** `matches!(err, S3Err::InvalidResponseErr(_))`; the last actual
   request is the `DELETE ...AbortMultipartUpload` (abort fired).

4. **`create_failure_maps_to_request_failed`** (non-2xx from create phase →
   RequestFailedErr). One `ReplayEvent`: create request → 403 AccessDenied XML.
   No upload exists to abort, so the error surfaces directly.
   **Asserts:** `matches!(err, S3Err::RequestFailedErr(_))`.

5. **`part_failure_triggers_abort`** (non-2xx from upload_part phase →
   RequestFailedErr + abort). Wire create_resp, a `upload_part` 403 AccessDenied
   XML, then abort request → 204. **Asserts:** `matches!(err, S3Err::RequestFailedErr(_))`;
   3 requests fired; request[2] is `DELETE` containing `x-id=AbortMultipartUpload`.

6. **`complete_failure_triggers_abort`** (non-2xx from complete phase →
   RequestFailedErr + abort). Wire create_resp, upload_part_resp(etag), a
   `complete` 403 AccessDenied XML, then abort request → 204. **Asserts:**
   `matches!(err, S3Err::RequestFailedErr(_))`; 4 requests fired; request[3] is
   `DELETE` containing `x-id=AbortMultipartUpload`.

7. **Size-routing tests** (in a new `pub mod routing` block — verifies
   `Store::put` picks the right path by file size). These exercise the M1 wiring:
   - **`small_file_routes_to_single_put`**: a body `<= PART_SIZE` (e.g.
     `b"tiny"`). Wire exactly one `PUT ...?x-id=PutObject` event returning 200.
     Call `store.put(src.to_file(), &obj("small.bin"))`. **Asserts:** exactly one
     request, a `PUT` with `x-id=PutObject` (no `uploads` / `UploadPart` query),
     i.e. it took the single-part branch.
   - **`large_file_routes_to_multipart`**: a body `> PART_SIZE`. To avoid a huge
     8 MiB+ fixture in the repo, build it in-process:
     `let big = vec![0u8; (PART_SIZE + 1024) as usize];` where
     `const PART_SIZE: u64 = 8 * 1024 * 1024;` is re-declared locally in the test
     (the crate constant is private). Write it via `temp_file_with(&big).await`.
     Wire create_resp, two `upload_part` events (`partNumber=1`, `partNumber=2`),
     and complete_resp. Call `store.put(src.to_file(), &obj("big.bin"))`.
     **Asserts:** first request is `POST ...uploads` (multipart branch taken), and
     a `PUT ...UploadPart&partNumber=1` request fired. (Two-part shape mirrors
     #102's `two_part_file` helper: `8 MiB + 1 KiB` → parts of 8 MiB and 1 KiB.)
     - Note: this fixture allocates ~8 MiB in memory for the test only; it is
       streamed off disk by the client, matching the thin-client contract. If the
       test suite's memory budget is a concern, keep this the single large-fixture
       test.

> Body-matching caveat (from the existing suite): streamed request bodies record
> as unbuffered `SdkBody` whose `.bytes()` is `None`, so `assert_requests_match`
> cannot byte-compare them. Assert **method + URI substrings** by hand via
> `replay.actual_requests()`, exactly as the existing `put` tests do. Use plain
> `ReplayEvent`s with `SdkBody::empty()` request bodies for the canned side.

**Verify:** `cargo test -p miru-agent --features test s3` passes (unit + integration).

## Milestone 4 — Coverage gate

- The existing gate `agent/src/s3/.covgate` contains `94.00` and applies to the
  **whole `agent/src/s3/` directory** (the covgate checker globs every source file
  whose path starts with the module dir, so `multipart.rs` is automatically
  included under the existing s3 gate). **No new `.covgate` file is needed** —
  `multipart.rs` inherits the 94% threshold.
- Ensure the new code clears 94% region coverage for the s3 module as a whole:
  - The two `InvalidResponseErr` producers are both directly tested (tests 2 & 3).
  - Every phase failure (create / part / complete) is tested (tests 4, 5, 6).
  - The happy path (create → part → complete) and both routing branches are
    tested (tests 1 & 7).
  - The three `part_size_for` branches are covered by the ported unit tests.
- Only genuinely-uncoverable arms should remain (there are none new here; unlike
  `map_sdk_err`'s non-exhaustive fallback, every multipart branch is reachable via
  replay). If coverage lands below 94%, add a targeted test rather than lowering
  the gate. Do **not** edit `agent/src/s3/.covgate` unless a documented,
  genuinely-unreachable branch forces it — and if so, note the exact branch.
- Regenerate/verify with `scripts/covgate.sh` (or `scripts/update-covgates.sh`
  only if intentionally raising the floor).

## Validation

All of the following must pass before the PR is ready. Run from the repo root
(`/home/ben/miru/workbench6/repos/agent`).

1. **Preflight reports `clean`:**
   ```sh
   scripts/preflight.sh
   ```
   This runs, in parallel: `scripts/lint.sh` (import linter + `cargo fmt` +
   machete + audit + clippy `-D warnings`), `scripts/covgate.sh` (tests under
   coverage instrumentation + per-module `.covgate` thresholds), and the tools
   lint/tests. The final line must read `Preflight clean`.

2. **Import linter passes in CHECK mode (no `--fix`):**
   ```sh
   cargo run --manifest-path tools/lint/Cargo.toml -- \
     --path agent/src --config .lint-imports.toml --assert-paths agent/tests
   ```
   Must report no findings. Requirements the new files must already satisfy
   (so CHECK mode is a no-op, not an autofix):
   - **Absolute `crate::` imports**, never `super::` — `multipart.rs` imports
     `crate::s3::{...}`, `crate::s3::errors::{...}`, `crate::filesys::...`,
     `crate::trace`. (The linter normalizes `super::` to `crate::`; writing
     `crate::` directly keeps CHECK mode clean.)
   - **Grouped same-crate imports** under the `// internal crates` /
     `// external crates` comment headers, in that order, blank-line separated
     (per AGENTS.md "Import ordering"). No `// standard crates` group is needed in
     `multipart.rs` after dropping the resume path's `std::collections::HashMap`.
   - Field-by-field-assert check on `agent/tests` (4+ `assert_eq!` on fields of the
     same variable). The multipart tests assert on request method/URI substrings,
     not many fields of one struct, so this should not trip; if it does, suppress
     with `// lint:allow(field-by-field-assert)` inside the offending test body.

3. **Targeted build/test sanity (fast inner loop while implementing):**
   ```sh
   cargo build -p miru-agent --features test
   cargo test  -p miru-agent --features test s3
   ```

## Definition of done

- `agent/src/s3/multipart.rs` exists with the stateless `put_multipart` path,
  `part_size_for`, and the create/upload_part/complete/abort primitives; two real
  `InvalidResponseErr` producers.
- `agent/src/s3/mod.rs` declares `pub mod multipart;`, defines `PART_SIZE`
  (`pub(crate)`), re-exports `{PartToUpload, Source, UploadedPart}`, and `put`
  routes by size.
- `agent/tests/s3/mod.rs` declares `pub mod multipart;`; `agent/tests/s3/multipart.rs`
  holds the 7 integration tests above using the shared replay harness.
- `agent/src/s3/.covgate` unchanged at `94.00`; s3 module coverage ≥ 94%.
- `scripts/preflight.sh` prints `Preflight clean`; the import linter passes in
  CHECK mode.
- Diff is a single focused PR (2/4); no resume/uploader code, no production caller.
