# ExecPlan: Split s3 into a thin stateless `Store` client + a stateful `uploader`

## Goal

The current `s3::Store` mixes two layers: S3 transport **and** durable resume
state (a local `upload_state_dir`, `UploadState` JSON persistence, resume/restart
policy). Separate them:

- **`s3::Store`** becomes a thin, **stateless** client over S3 — multipart
  primitives + streaming from `File` + error mapping. No filesystem dependency,
  no `Dir`, no durable state.
- **`s3::uploader`** (new module) is built *on top of* `Store` and owns all the
  durable-resume concerns: the `upload_state_dir`, the `UploadState` record, and
  the resume-vs-restart decision.

Seam confirmed with the user (the "recommended" option): the client keeps a
**stateless** `put_multipart` convenience; the uploader drives the resumable path
via the client's primitives.

Context: `Store` has **no production caller yet**, so this is a pure internal
reshaping with no external breakage. Current code is at commit `4504a05`
(resumable uploads live inside `Store`); this plan moves that state up.

## Target `s3::Store` (thin client) — `agent/src/s3/mod.rs`

**Remove from the client:** `Config.upload_state_dir`, the `Store.upload_state_dir`
field, the `UploadState` record, `ResumeState`, `upload_state_file`,
`resume_or_restart`, and the state persist/delete calls. Also drop the now-unused
`use crate::filesys::dir::Dir;`, `WriteOptions`, `serde`, and `sanitize_filename`
imports from this file (they move to the uploader).

**`Config`** → `{ creds, region, bucket }` (no state dir).

**Keep `Options { part_size }`** as the single-vs-multipart threshold for the
client's `put` convenience.

**Public multipart primitives** (new — extracted from the current internals):
- `pub async fn create_multipart_upload(&self, key: &str) -> Result<String, S3Err>`
  — returns the `upload_id`; `InvalidResponseErr` if S3 omits it.
- `pub async fn upload_part(&self, key: &str, upload_id: &str, part_number: i32, file: &File, offset: u64, len: u64) -> Result<String, S3Err>`
  — streams one chunk via `ByteStream::read_from().path(file.path()).offset().length(Length::Exact(len))`, returns the part ETag; `InvalidResponseErr` if the response omits the ETag.
- `pub async fn list_parts(&self, key: &str, upload_id: &str) -> Result<Vec<PartInfo>, S3Err>`
  — paginated (`is_truncated()`/`next_part_number_marker()`); returns parts that
  have all of `{part_number, etag, size}`. A 404 (`NoSuchUpload`) must still be
  distinguishable by the uploader — return it as a typed signal: prefer
  `Result<Option<Vec<PartInfo>>, S3Err>` where `Ok(None)` means "no such upload"
  (404), or add an `S3Err` predicate. **Decision: return `Result<Option<Vec<PartInfo>>, S3Err>`**
  — `Ok(None)` on 404 (mirror the existing `get`/`exists` raw_response status==404
  pattern), `Ok(Some(parts))` otherwise.
- `pub async fn complete_multipart_upload(&self, key: &str, upload_id: &str, parts: &[(i32, String)]) -> Result<(), S3Err>`
  — builds `CompletedPart`/`CompletedMultipartUpload` internally (don't leak aws types across the API).
- `pub async fn abort_multipart_upload(&self, key: &str, upload_id: &str) -> Result<(), S3Err>`
  — returns `Result` (callers decide whether to treat it best-effort).

Add a small public type:
```rust
/// One already-uploaded part as reported by S3 ListParts.
pub struct PartInfo {
    pub part_number: i32,
    pub etag: String,
    pub size: u64,
}
```

**Stateless convenience (kept):**
- `pub async fn put_multipart(&self, key: &str, file: &File, size: u64) -> Result<(), S3Err>`
  — `create_multipart_upload` → loop `upload_part` over `part_size_for(size)`
  chunks, collecting `(part_number, etag)` → `complete_multipart_upload`; on any
  error, best-effort `abort_multipart_upload`, then propagate. **No durable state.**
- `pub async fn put(&self, key: &str, file: &File) -> Result<(), S3Err>` — unchanged
  routing: `if size > self.opts.part_size { self.put_multipart(...) } else { self.put_object(...) }`.
- Rename the current private `put_singlepart` → `pub async fn put_object(&self, key, file: &File)`.

**Keep unchanged:** `get`, `delete`, `exists`, `map_body_io_err`,
`map_bytestream_err`, and the `part_size_for` math. Make part planning reachable
by the uploader: `pub(crate) fn part_size_for(size: u64) -> u64` (or a
`pub(crate) fn part_plan(size) -> impl Iterator<Item=(i32 /*part_number*/, u64 /*offset*/, u64 /*len*/)>`).
**Decision: expose `pub(crate) fn part_size_for`** and let the uploader do the
offset loop (keeps the surface minimal).

**Module doc:** trim the resumable paragraph from the client doc (move it to the
uploader); keep the "distinct from `crate::disk`" and streaming notes. Ensure no
stale intra-doc links.

## New module `s3::uploader` — `agent/src/s3/uploader.rs`

Add `pub mod uploader;` to `agent/src/s3/mod.rs`.

```rust
/// Durable handle to an in-progress multipart upload, persisted so a reboot can
/// resume. S3 (ListParts) is the source of truth for landed parts; this records
/// only the upload_id plus guards to detect a changed source file.
#[derive(Debug, Serialize, Deserialize)]
struct UploadState { upload_id: String, key: String, size: u64, part_size: u64 }

/// Resumable multipart uploads layered over a thin `Store`. Owns the durable
/// upload-id state directory and the resume-vs-restart policy.
pub struct Uploader {
    store: Store,
    state_dir: Dir,   // required — resumability is the whole point of this type
}

impl Uploader {
    pub fn new(store: Store, state_dir: Dir) -> Self { ... }
    pub fn store(&self) -> &Store { &self.store }   // for callers needing get/delete/exists

    /// Uploads `file` to `key`, resuming an interrupted multipart upload if durable
    /// state exists. Small files go straight through `store.put_object`.
    pub async fn upload(&self, key: &str, file: &File) -> Result<(), S3Err> { ... }
}
```

`upload` logic (moved from the old `Store::put_multipart` + `resume_or_restart`):
1. `size = file.size().await?`.
2. If `size <= self.store.multipart_threshold()` → `self.store.put_object(key, file).await` (no state). Add `pub fn multipart_threshold(&self) -> u64 { self.opts.part_size }` accessor on `Store`.
3. Else multipart with resume:
   - `part_size = Store::part_size_for(size)`.
   - `state_file = self.state_dir.file(&format!("{}.json", sanitize_filename(key)))`.
   - `resume_or_restart(&state_file, key, size, part_size)` → `Option<ResumeState { upload_id, completed_parts: Vec<(i32,String)>, offset, next_part_number }>`:
     - no state file → `None`.
     - unreadable/corrupt state → best-effort delete → `None`.
     - `state.key/size/part_size` mismatch → best-effort `store.abort_multipart_upload` + delete → `None`.
     - `store.list_parts(...)` → `Ok(None)` (404/expired) → delete → `None`;
       `Ok(Some(parts))` → build contiguous prefix 1..N (each present, size == part_size), `offset = N*part_size`, `next_part_number = N+1` → `Some`.
   - fresh (`None`): `upload_id = store.create_multipart_upload(key)` → persist
     `UploadState` via `state_file.write_json(&s, WriteOptions::OVERWRITE_ATOMIC)`
     → seed empty parts / offset 0 / part 1.
   - loop remaining parts: `etag = store.upload_part(key, &upload_id, part_number, file, offset, len)`, push `(part_number, etag)`, advance.
   - `store.complete_multipart_upload(key, &upload_id, &parts)` → on success
     `state_file.delete()`.
   - on any error in the loop/complete: best-effort `store.abort_multipart_upload`
     + `state_file.delete()`, propagate the error.

Imports here: `crate::filesys::{dir::Dir, file::File, file::sanitize_filename, WriteOptions}`,
`serde::{Serialize, Deserialize}`, `super::{Store, PartInfo, S3Err}` (+ error variants
as needed). Follow std/internal/external grouping.

## `agent/src/s3/errors.rs`
No new variant expected — `FileSysErr` (state IO), `InvalidResponseErr`
(malformed S3 responses), and `map_sdk_err_common` cover everything, as today.

## Tests — `agent/tests/s3/mod.rs`

- Existing client tests (put/get/delete/exists/abort/construction/error mapping):
  update `Config` literals to drop `upload_state_dir` (no more field). The
  `store_with`/`store_with_part_size` helpers lose the state-dir field. These
  tests otherwise stay as-is (they already exercise the stateless client).
- **Move the resumable tests** (`pub mod resumable`) to drive an `Uploader`
  instead of a stateful `Store`: build a `Store` (via `from_http_client`) + a
  `tempfile::TempDir`, wrap in `Uploader::new(store, Dir::new(tmp.path()))`, and
  call `uploader.upload(key, &File::new(...))`. Keep the same scenarios and the
  16 MiB fixture (two 8 MiB parts). `use miru_agent::s3::uploader::Uploader;` and
  `use miru_agent::filesys::dir::Dir;`. Keep the TempDir alive in the test body.
- Optional: add focused primitive tests (create_multipart_upload returns id;
  upload_part returns etag; list_parts paginates / maps 404→`Ok(None)`) if needed
  to hold the s3 covgate. Reuse existing canned-XML helpers.
- New `s3::uploader.rs` is a **file** inside the `s3/` module dir, so it is
  covered by the existing `agent/src/s3/.covgate` (no new .covgate file needed).

## Test steps
1. Client primitives: `create_multipart_upload`/`upload_part`/`complete`/`abort`
   round-trips fire the expected requests; `list_parts` paginates and maps a 404
   to `Ok(None)`.
2. Client `put_multipart` (stateless) still does create→parts→complete and
   aborts on failure — no `list_parts`, no state file.
3. `put`/`get`/`delete`/`exists` unchanged and green.
4. `Uploader::upload` resume scenarios (via primitives): resume-skips-landed-parts,
   expired-id-restarts, changed-source-restarts, fresh-persists-then-cleans-up,
   failure-aborts-and-removes-state.
5. The 3 `part_size_for` unit tests unchanged.

## Validation
- Build: `cargo build --features test -p miru-agent`.
- Tests: `./scripts/test.sh` (`RUST_LOG=off cargo test --features test`) — all pass.
- Lint: `scripts/lint.sh` (import linter, `cargo fmt --check`, machete/diet, audit,
  clippy `-D warnings`; watch intra-doc links after moving docs).
- Coverage: `scripts/covgate.sh` — `agent/src/s3/.covgate` (88.00%) must pass; the
  moved resume code stays covered by the uploader tests. (The `workers` covgate
  shortfall pre-exists on `main` and is out of scope.)
- **Preflight must report `clean` before the changes are pushed.**
