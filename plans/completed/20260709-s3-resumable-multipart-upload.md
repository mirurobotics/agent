# ExecPlan: Resumable S3 multipart upload (PR 3/4 of #102 split)

## Context

`agent/src/s3/` (Rust crate `miru-agent`) has a stateless multipart upload path
merged on `main` (commit `9060022`, "s3 multipart upload (2/4 — split of #102)").
`put_multipart` creates a fresh upload each call and aborts it best-effort on any
in-process failure. This PR adds a **resumable** entry point: given an existing
`upload_id`, list the parts that already landed in S3, upload only the missing
parts, and complete — **without ever aborting**, so a resume can be retried after
a crash/power-off. This is PR 3/4 of splitting the original #102
(`origin/feat/s3-object-storage-crud`).

Scope is deliberately narrow. This PR carves the resume API and its `ListParts`
plumbing out of #102. It does **not** include the durable local `upload_id`
persistence layer (`UploadState` JSON files, `upload_state_dir` config) from the
larger #102 plan — that is out of scope here. The caller supplies the
`upload_id`; how it is persisted is another PR's concern.

Work happens on branch `feat/s3-resumable-multipart` off `main`.

### Source to carve from

`origin/feat/s3-object-storage-crud` (#102) has the reference implementation.
Read it with `git show`:

- `git show origin/feat/s3-object-storage-crud:agent/src/s3/multipart.rs`
  — `resume_multipart_upload`, `list_parts`, `list_parts_page`.
- `git show origin/feat/s3-object-storage-crud:agent/src/s3/errors.rs`
  — `NoSuchUploadErr` struct + `Error` impl + enum arm + `impl_error!` entry.
- `git show origin/feat/s3-object-storage-crud:agent/tests/s3/multipart.rs`
  — the `resume` test module (lines ~318+).

**#102 targets an OLDER module shape.** The mechanical adaptations to current
`main` are the bulk of this plan's care; see "Key adaptations" below.

### Key adaptations (#102 → main)

| #102 (old) | main (this PR) |
| --- | --- |
| `self.map_bytestream_err(...)`, `errors::map_sdk_err_common(...)` | free-fn mappers `errors::map_sdk_err(...)`; `map_body_io_err` for local reads |
| `upload_part(&self, src, dst, &PartToUpload) -> Result<ETag>` (streams via `ByteStream::read_from().offset().length()`) | main's existing `upload_part(&self, src, dst, upload_id, part_number, offset, length) -> Result<CompletedPart>` (**buffered** via `read_part_bytes`) — reuse as-is |
| `PartToUpload` / `UploadedPart` structs, `type ETag = String` | none — main uses `CompletedPart` directly and a `(part_number, offset, length)` tuple plan. Do **not** introduce these structs. |
| `complete_multipart_upload(&self, obj, upload_id, &[UploadedPart])` (builds `CompletedPart`s internally) | main's `complete_multipart_upload(&self, obj, upload_id, &[CompletedPart])` — pass `CompletedPart`s directly |
| resume recomputes part ranges inline (`(number-1)*part_size`, `div_ceil`) | reuse main's `Self::part_plan(size) -> Vec<(i32, u64, u64)>` — single source of truth for part boundaries |
| `super::` / mixed imports | absolute `crate::s3::...` imports, grouped std/internal/external per `AGENTS.md` |
| `errors::InvalidResponseErr { ... }` inline in `upload_part` | already handled by main's `upload_part` (uses `missing_response_field`) — untouched |

### Confirmed semantics

- **No abort on resume failure.** `resume_multipart_upload` never calls
  `abort_multipart_upload`. A resumable upload is deliberately left intact on
  failure so it can be retried. (Confirmed against #102: `resume_multipart_upload`
  has no abort call anywhere; contrast `put_multipart`, which aborts.)
- **`NoSuchUploadErr`**: `Code::ResourceNotFound`, `http_status` `NOT_FOUND` (404).
  (Confirmed against #102's `impl crate::errors::Error for NoSuchUploadErr`.)
- **`list_parts` return type**: `Result<Option<Vec<CompletedPart>>, S3Err>`.
  `Ok(None)` means S3 no longer knows the upload (a 404 / `NoSuchUpload`), which
  the caller maps to `NoSuchUploadErr`. `Ok(Some(parts))` is the (possibly empty)
  landed set. This mirrors #102's `Option<Vec<UploadedPart>>` but carries
  `CompletedPart` to match main's types. (#102 returned `UploadedPart`; we adapt
  to `CompletedPart` since main has no `UploadedPart`.)

## Goal

Add `Store::resume_multipart_upload(&self, src: &Source, dst: &Object, upload_id: &str)
-> Result<(), S3Err>` plus its private `list_parts` / `list_parts_page` helpers and
a new `S3Err::NoSuchUploadErr` variant, adapted to current `main`. `put_multipart`
and every other module stay byte-for-byte unchanged. Thin public surface: only
`resume_multipart_upload` is public; `list_parts`/`list_parts_page` are private.

## Milestones

1. **Errors**: add `S3Err::NoSuchUploadErr` (struct + `Error` impl + enum arm +
   `impl_error!` entry + unit test).
2. **Resume API**: add `resume_multipart_upload` + private `list_parts` /
   `list_parts_page` to `multipart.rs`, reusing `part_plan`, `upload_part`,
   `complete_multipart_upload` unchanged.
3. **Tests**: add a `resume` test module to `tests/s3/multipart.rs` with the
   `ListParts` req/resp/shape helpers and the four scenarios.
4. **Validation**: `scripts/preflight.sh` clean, then CI-parity check-mode lint
   (`LINT_FIX=0 scripts/lint.sh`) exits 0; commit any lint fixes (SSH-signed).

---

## Steps

### `agent/src/s3/errors.rs`

Add the new leaf error type near `ObjectNotFoundErr` (top of the file, before
`ConnectionErr`):

```rust
#[derive(Debug, thiserror::Error)]
#[error("no such multipart upload '{upload_id}' for object '{key}'")]
pub struct NoSuchUploadErr {
    pub key: String,
    pub upload_id: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for NoSuchUploadErr {
    fn code(&self) -> Code {
        Code::ResourceNotFound
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::NOT_FOUND
    }
}
```

Add the enum arm (after `ObjectNotFoundErr`):

```rust
    #[error(transparent)]
    NoSuchUploadErr(NoSuchUploadErr),
```

Add the `impl_error!` entry (after `ObjectNotFoundErr`):

```rust
crate::impl_error!(S3Err {
    ObjectNotFoundErr,
    NoSuchUploadErr,
    ConnectionErr,
    RequestFailedErr,
    InvalidResponseErr,
    LocalIoErr,
    FileSysErr,
});
```

No new mapper free-fn is needed: `is_not_found` already exists on `main` and is
reused by `list_parts_page`. Keep doc comments to the single `#[error(...)]`
one-liner — no rationale block on the struct.

### `agent/src/s3/multipart.rs`

**Imports.** Add to the internal-crates group the new error type; add to the
external-crates group the `ListPartsOutput` type. Keep the existing grouping
(std / internal / external) and absolute `crate::s3::` paths:

```rust
// internal crates
use crate::filesys::{file::File, path::PathExt};
use crate::s3::errors::NoSuchUploadErr;
use crate::s3::{errors, Object, S3Err, Store, PART_SIZE};

// external crates
use aws_sdk_s3::operation::list_parts::ListPartsOutput;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
```

`trace!` is `#[macro_export]` (defined in `agent/src/errors/mod.rs`), so
`crate::trace!()` resolves crate-wide **without** an import. Do **not** add
`use crate::trace;` — main's `multipart.rs` has no such import and an unused one
would fail check-mode lint. Call it fully-qualified as `crate::trace!()` (as the
snippets below do).

**`resume_multipart_upload`** — public, added inside `impl Store` (place it right
after `put_multipart` so the two upload entry points sit together):

```rust
/// Resumes an existing multipart upload: lists the parts that already landed
/// in S3, uploads only the missing parts, and completes. Never aborts, so a
/// resume is safe to retry.
///
/// `NoSuchUploadErr` if S3 no longer knows the upload (expired or aborted).
/// The caller must resume against the same file bytes the upload was started
/// for: part `n` is the byte range `part_plan` assigns from `src.size`, and a
/// landed part's ETag is trusted as-is rather than re-hashed.
pub async fn resume_multipart_upload(
    &self,
    src: &Source,
    dst: &Object,
    upload_id: &str,
) -> Result<(), S3Err> {
    // Index the already-landed parts by number. `None` means S3 no longer
    // knows this upload, so it cannot be resumed.
    let landed: std::collections::HashMap<i32, CompletedPart> =
        match self.list_parts(dst, upload_id).await? {
            Some(parts) => parts
                .into_iter()
                .filter_map(|p| p.part_number().map(|n| (n, p)))
                .collect(),
            None => {
                return Err(S3Err::NoSuchUploadErr(NoSuchUploadErr {
                    key: dst.key.to_string(),
                    upload_id: upload_id.to_string(),
                    trace: crate::trace!(),
                }));
            }
        };

    // Walk the full plan in order, reusing a landed part's CompletedPart or
    // uploading the missing range. `part_plan` is the single source of truth
    // for part boundaries, shared with `put_multipart`.
    let mut parts: Vec<CompletedPart> = Vec::new();
    for (part_number, offset, len) in Self::part_plan(src.size) {
        let part = match landed.get(&part_number) {
            Some(existing) => existing.clone(),
            None => {
                self.upload_part(&src.file, dst, upload_id, part_number, offset, len)
                    .await?
            }
        };
        parts.push(part);
    }

    self.complete_multipart_upload(dst, upload_id, &parts).await
}
```

Notes:
- `part_plan` is currently a private `fn` (not `pub(crate)`); it stays private
  and is called from the same module, so no visibility change.
- `upload_part` and `complete_multipart_upload` are reused unchanged. Both are
  already private methods on `Store` in the same file.
- Landed parts already carry `(part_number, etag)` as `CompletedPart` from
  `list_parts`; reuse the value directly (no re-wrapping).

**`list_parts`** — private, paginated accumulator. Returns `Ok(None)` on a
missing upload:

```rust
/// Lists every part already uploaded for `upload_id`, following pagination.
/// `Ok(None)` when S3 reports the upload no longer exists (404 / NoSuchUpload),
/// distinguishing an expired upload from an empty listing.
async fn list_parts(
    &self,
    obj: &Object,
    upload_id: &str,
) -> Result<Option<Vec<CompletedPart>>, S3Err> {
    let mut parts: Vec<CompletedPart> = Vec::new();
    let mut marker: Option<String> = None;

    loop {
        let Some(page) = self
            .list_parts_page(obj, upload_id, marker.as_deref())
            .await?
        else {
            return Ok(None);
        };

        parts.extend(page.parts().iter().filter_map(|part| {
            let number = part.part_number()?;
            let etag = part.e_tag()?;
            Some(CompletedPart::builder().part_number(number).e_tag(etag).build())
        }));

        match page.next_part_number_marker() {
            Some(next) if page.is_truncated() == Some(true) => marker = Some(next.to_string()),
            _ => return Ok(Some(parts)),
        }
    }
}
```

**`list_parts_page`** — private, one `ListParts` call, `NoSuchUpload` via
raw-status → `Ok(None)` (mirrors `get`/`exists` using `errors::is_not_found`):

```rust
/// Fetches one page of [`Self::list_parts`], resuming after `marker` when given.
/// `Ok(None)` if S3 reports the upload no longer exists (404 / NoSuchUpload),
/// mirroring the raw-response status check `get`/`exists` use; any other SDK
/// error propagates via `map_sdk_err`.
async fn list_parts_page(
    &self,
    obj: &Object,
    upload_id: &str,
    marker: Option<&str>,
) -> Result<Option<ListPartsOutput>, S3Err> {
    let mut req = self
        .client
        .list_parts()
        .bucket(&obj.bucket)
        .key(&obj.key)
        .upload_id(upload_id);
    if let Some(marker) = marker {
        req = req.part_number_marker(marker);
    }

    match req.send().await {
        Ok(page) => Ok(Some(page)),
        Err(err) if errors::is_not_found(&err) => Ok(None),
        Err(err) => Err(errors::map_sdk_err(
            "list_parts",
            Some(obj.key.to_string()),
            err,
        )),
    }
}
```

Keep all three doc comments to the lean one-liner style shown. Do **not** add
multi-paragraph rationale/precondition blocks in the source — put the
"same-file-bytes" caveat in the PR description, keeping only the two-line note in
`resume_multipart_upload`'s doc comment above.

**Do not touch** `put_multipart`, `part_size_for`, `part_plan`,
`create_multipart_upload`, `exec_multipart_upload`, `upload_parts`,
`read_part_bytes`, `upload_part`, `complete_multipart_upload`,
`abort_multipart_upload`, or any of the existing `#[cfg(test)] mod tests`
unit tests.

### `agent/src/s3/mod.rs`

No change required: `resume_multipart_upload` is a method on the already-exported
`Store`, and `NoSuchUploadErr` reaches callers through the exported `S3Err` enum.
`Source` is already re-exported. Leave `mod.rs` untouched.

---

## Test steps — `agent/tests/s3/multipart.rs`

Add a `pub mod resume` sibling to the existing `pub mod put`. It carries its own
`ListParts` helpers (mirroring the existing `create_*`/`upload_part_*`/`complete_*`
helpers) plus the four scenarios. The test harness (`store_with`, `obj`,
`actual_shapes`, `uri`, `temp_file_with`, `BUCKET`, `UPLOAD_ID`, `Source`,
`source_of`, `File`, `files`) is already in scope via `use super::*;`.

### Wire details to pin

- **ListParts request**: `GET /<key>?uploadId=<id>` (SDK appends `&x-id=ListParts`).
  Paginated pages add `part-number-marker=<n>`. The SDK emits query params in a
  fixed order; match #102's exact strings:
  - page 1: `big.bin?uploadId={UPLOAD_ID}&x-id=ListParts`
  - page 2 (after marker 1): `big.bin?part-number-marker=1&uploadId={UPLOAD_ID}&x-id=ListParts`
- **ListParts response**: `<ListPartsResult>` XML with zero or more
  `<Part><PartNumber>N</PartNumber><ETag>"..."</ETag><Size>S</Size></Part>` and
  either `<IsTruncated>false</IsTruncated>` or
  `<IsTruncated>true</IsTruncated><NextPartNumberMarker>M</NextPartNumberMarker>`.
- **upload_part / complete** reuse the existing module helpers
  (`upload_part_req`/`upload_part_shape`, `complete_req`/`complete_shape`) — but
  note the module's `complete_shape()` on `main` is `POST big.bin?uploadId={UPLOAD_ID}`
  (no `x-id`). Reuse those `main` helpers rather than #102's `x-id=CompleteMultipartUpload`
  variant, so the resume tests match `main`'s wire shape. The `upload_part_*` and
  `complete_*` helpers are `pub(crate)` free fns at the top of the `multipart`
  test module; the `resume` submodule reaches them via `use super::*;` (same
  pattern the `put` submodule already uses). The `list_parts_*` helpers are new
  and live inside the `resume` module.

### Canned ListParts XML helper

```rust
/// Canned `ListPartsResult` XML for `parts` (`(number, etag, size)`), optionally
/// truncated with a `NextPartNumberMarker`.
fn list_parts_xml(parts: &[(i32, &str, u64)], next_marker: Option<i32>) -> String {
    let mut body = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<ListPartsResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Bucket>test-bucket</Bucket><Key>big.bin</Key><UploadId>test-upload-id</UploadId>"#,
    );
    match next_marker {
        Some(m) => body.push_str(&format!(
            "<IsTruncated>true</IsTruncated><NextPartNumberMarker>{m}</NextPartNumberMarker>"
        )),
        None => body.push_str("<IsTruncated>false</IsTruncated>"),
    }
    for (number, etag, size) in parts {
        body.push_str(&format!(
            "<Part><PartNumber>{number}</PartNumber><ETag>{etag}</ETag><Size>{size}</Size></Part>"
        ));
    }
    body.push_str("</ListPartsResult>");
    body
}

fn list_parts_resp(parts: &[(i32, &str, u64)], next_marker: Option<i32>) -> http::Response<SdkBody> {
    http::Response::builder()
        .status(200)
        .header("content-type", "application/xml")
        .body(SdkBody::from(list_parts_xml(parts, next_marker)))
        .unwrap()
}

fn list_parts_req() -> http::Request<SdkBody> {
    http::Request::builder()
        .method("GET")
        .uri(uri(&format!("big.bin?uploadId={UPLOAD_ID}&x-id=ListParts")))
        .body(SdkBody::empty())
        .unwrap()
}

fn list_parts_shape() -> (String, String) {
    shape("GET", &format!("big.bin?uploadId={UPLOAD_ID}&x-id=ListParts"))
}
```

A two-part source file (8 MiB + 1 KiB ⇒ parts `(1, 0, 8 MiB)` and `(2, 8 MiB, 1 KiB)`):

```rust
async fn two_part_file() -> files::TempFile {
    const PART_SIZE: u64 = 8 * 1024 * 1024;
    let bytes = vec![7u8; (PART_SIZE + 1024) as usize];
    temp_file_with(&bytes).await
}
```

(Use the crate's `temp_file_with`/`source_of` helpers already in the test module;
build the `Source` via `source_of(&two_part_file().await).await` so `src.size` is
read off disk the same way `put` tests do.)

### Scenario (a) — resume skips already-landed parts

`ListParts` reports part 1 landed; assert the wire sequence is
`list_parts → upload_part(2) → complete` (part 1 is **not** re-uploaded), and the
complete manifest carries both etags in ascending order.

```rust
#[tokio::test]
async fn resume_skips_landed_parts() {
    let src = source_of(&two_part_file().await).await;

    let (store, replay) = store_with(vec![
        ReplayEvent::new(
            list_parts_req(),
            list_parts_resp(&[(1, "\"landed-1\"", 8 * 1024 * 1024)], None),
        ),
        ReplayEvent::new(upload_part_req(2), upload_part_resp("\"fresh-2\"")),
        ReplayEvent::new(complete_req(), complete_resp()),
    ]);

    store
        .resume_multipart_upload(&src, &obj("big.bin"), UPLOAD_ID)
        .await
        .unwrap();

    // list_parts → upload part 2 only → complete. Part 1 is never re-uploaded.
    assert_eq!(
        actual_shapes(&replay),
        vec![list_parts_shape(), upload_part_shape(2), complete_shape()]
    );

    // Complete manifest lists both parts in order with the landed + fresh etags.
    let requests = replay.actual_requests().collect::<Vec<_>>();
    let manifest = std::str::from_utf8(
        requests.last().unwrap().body().bytes().expect("in-memory complete body"),
    )
    .unwrap();
    let p1 = manifest.find("<PartNumber>1</PartNumber>").expect("part 1 listed");
    let p2 = manifest.find("<PartNumber>2</PartNumber>").expect("part 2 listed");
    assert!(p1 < p2, "parts ascending");
    assert!(manifest.contains("landed-1"), "part 1 reuses its landed etag");
    assert!(manifest.contains("fresh-2"), "part 2 carries the freshly-uploaded etag");
}
```

### Scenario (b) — expired / nonexistent upload → `NoSuchUploadErr`

`ListParts` 404s with `NoSuchUpload`; assert `NoSuchUploadErr` and that only the
`list_parts` request fired (no upload_part, no complete).

```rust
#[tokio::test]
async fn resume_expired_upload_maps_to_no_such_upload() {
    let src = source_of(&two_part_file().await).await;

    let not_found = http::Response::builder()
        .status(404)
        .header("content-type", "application/xml")
        .body(SdkBody::from(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<Error><Code>NoSuchUpload</Code><Message>The specified upload does not exist.</Message></Error>"#,
        ))
        .unwrap();

    let (store, replay) = store_with(vec![ReplayEvent::new(list_parts_req(), not_found)]);

    let err = store
        .resume_multipart_upload(&src, &obj("big.bin"), UPLOAD_ID)
        .await
        .unwrap_err();

    assert!(matches!(err, S3Err::NoSuchUploadErr(_)));
    assert!(matches!(err.code(), Code::ResourceNotFound));
    assert_eq!(err.http_status().as_u16(), 404);

    // Only the list_parts call fired — no upload, no complete (and crucially no abort).
    assert_eq!(actual_shapes(&replay), vec![list_parts_shape()]);
}
```

(`err.code()` / `err.http_status()` require `Error` in scope; the test crate's
`mod.rs` already imports `miru_agent::errors::{Code, Error}`.)

### Scenario (c) — list_parts pagination merges two pages

Two `ListParts` pages (`IsTruncated=true` w/ `NextPartNumberMarker=1`, then
`false`) supply both parts as landed; resume uploads nothing and completes.
Asserts the second page carries `part-number-marker=1` and that no `UploadPart`
fired.

```rust
#[tokio::test]
async fn resume_merges_paginated_list_parts() {
    let src = source_of(&two_part_file().await).await;

    let page2_req = http::Request::builder()
        .method("GET")
        .uri(uri(&format!(
            "big.bin?part-number-marker=1&uploadId={UPLOAD_ID}&x-id=ListParts"
        )))
        .body(SdkBody::empty())
        .unwrap();

    let (store, replay) = store_with(vec![
        ReplayEvent::new(
            list_parts_req(),
            list_parts_resp(&[(1, "\"landed-1\"", 8 * 1024 * 1024)], Some(1)),
        ),
        ReplayEvent::new(page2_req, list_parts_resp(&[(2, "\"landed-2\"", 1024)], None)),
        ReplayEvent::new(complete_req(), complete_resp()),
    ]);

    store
        .resume_multipart_upload(&src, &obj("big.bin"), UPLOAD_ID)
        .await
        .unwrap();

    let requests = replay.actual_requests().collect::<Vec<_>>();
    assert_eq!(requests.len(), 3); // two list pages, then complete
    assert!(requests[1].uri().to_string().contains("part-number-marker=1"));
    assert!(
        !requests.iter().any(|r| r.uri().to_string().contains("x-id=UploadPart")),
        "all parts landed via the listing — nothing re-uploaded"
    );
}
```

### Optional scenario (d) — none landed uploads both parts

If coverage gates require the empty-listing branch, add a test where `ListParts`
returns an empty listing and resume uploads parts 1 and 2 then completes
(sequence `list_parts → upload_part(1) → upload_part(2) → complete`). Mirror
#102's `uploads_all_parts_when_none_landed`. Include only if covgate flags the
`landed.get(&part_number) == None` path as uncovered (scenario (a) already covers
one uploaded + one landed part, so it likely is covered; confirm with covgate).

### Error unit test — `agent/src/s3/errors.rs`

Add to the `error_types` submodule of `mod tests`, alongside
`object_not_found_maps_to_resource_not_found`:

```rust
#[test]
fn no_such_upload_maps_to_resource_not_found() {
    let err = S3Err::NoSuchUploadErr(NoSuchUploadErr {
        key: "k".to_string(),
        upload_id: "u".to_string(),
        trace: crate::trace!(),
    });
    assert!(matches!(err.code(), Code::ResourceNotFound));
    assert_eq!(err.http_status().as_u16(), 404);
    assert!(!err.is_network_conn_err());
    assert!(err.to_string().contains("no such multipart upload"));
}
```

---

## Validation

1. `scripts/preflight.sh` — must print "Preflight clean" (runs lint + covgate for
   both `miru-agent` and the tools crate, in parallel). Iterate until clean.
2. **CI-parity lint (check mode, no `--fix`)**: run
   `LINT_FIX=0 scripts/lint.sh` and confirm it exits 0. `preflight.sh` runs lint
   in fix mode (`LINT_FIX` defaults to 1), which silently rewrites imports;
   CI runs check mode and will fail if any import ordering / grouping was left
   unfixed. If check mode reports findings, apply them (absolute `crate::`
   imports, grouped same-crate imports per `.lint-imports.toml` and `AGENTS.md`),
   re-run, and commit the fixes.
3. Confirm the new tests pass:
   `cargo test --package miru-agent --all-features s3::multipart::resume`
   and the errors unit test
   `cargo test --package miru-agent s3::errors::tests::error_types::no_such_upload`.
4. Confirm `put_multipart` and the pre-existing multipart tests still pass
   unchanged (no regressions in `s3::multipart::put` / `s3::put::routing`).

## Constraints

- **SSH-signed commits** — never disable `commit.gpgsign`.
- **Lean doc comments** — concise one-liners only; no multi-paragraph
  rationale/limitation blocks in source. Caveats (same-file-bytes precondition,
  no-abort rationale) go in the PR description, not the code.
- **Thin client** — minimal public surface: `resume_multipart_upload` public;
  `list_parts` / `list_parts_page` private. No new public types beyond
  `NoSuchUploadErr` (reached via the `S3Err` enum).
- **Do not touch** `put_multipart` or any unrelated module. No `mod.rs` changes.
- **CI-parity** — preflight clean AND check-mode import lint exit 0, with fixes
  committed.
```
