# ExecPlan: Resumable S3 multipart uploads across reboots

## Goal

Make multipart uploads in `agent/src/s3/` survive a power-off and resume instead
of re-uploading from byte 0. S3 `ListParts` is the source of truth for which
parts already landed; the only durable local state is the `upload_id` (plus a
small guard record), persisted atomically to disk and keyed by object key.

Scope confirmed with the user: **full implementation** (discovery + recovery +
resume loop + cleanup + fall-back-to-restart), with the **upload_id persisted
locally** (not rediscovered via `ListMultipartUploads`).

## Design summary

- The multipart chunk size is `part_size_for(size)` (deterministic from size,
  const-based, unchanged). `Options.part_size` remains only the single-vs-multipart
  threshold. Resume correctness relies on: same source file bytes (⇒ same size)
  ⇒ same chunk size ⇒ part N maps to the identical byte range.
- Durable state = one JSON file per in-progress upload: `{ upload_id, key, size,
  part_size }`. Written right after `create_multipart_upload`, deleted on
  successful `complete` **and** on `abort`. A power-off runs neither delete, so
  the file survives and the next run resumes.
- Resumability is **opt-in via a configured directory**. `Config` gains
  `upload_state_dir: Option<Dir>`. `None` ⇒ current stateless behavior (create
  fresh every time, abort on error); `Some(dir)` ⇒ persist + resume. Production
  will pass `Some(<layout subdir>)`; the existing tests pass `None` (no behavior
  change).
- **In-process errors still abort** (best-effort) + delete the state file, exactly
  as today. Resume targets crash/power-off, where the abort path never executes.

## Files changed

### `agent/src/s3/mod.rs`

**Imports / types**
- Add `use crate::filesys::dir::Dir;`, `use crate::filesys::{WriteOptions};` (and
  `file::sanitize_filename` — confirm the exact re-export path; `sanitize_filename`
  is a free fn in `filesys::file`). Keep import grouping (std/internal/external).
- Add `use serde::{Serialize, Deserialize};`.
- Add a serde record:
  ```rust
  /// Durable handle to an in-progress multipart upload, persisted so a reboot can
  /// resume instead of restarting. S3 (ListParts) is the source of truth for which
  /// parts landed; this only records the upload_id plus guards to detect a changed
  /// source file.
  #[derive(Debug, Serialize, Deserialize)]
  struct UploadState {
      upload_id: String,
      key: String,
      size: u64,
      part_size: u64,
  }
  ```

**`Config`** — add field `pub upload_state_dir: Option<Dir>`.

**`Store`** — add field `upload_state_dir: Option<Dir>`; set it from `cfg.upload_state_dir`
in both `new` and `from_http_client`.

**`put_multipart` rework** (replaces the current create-then-upload body):
1. `let part_size = Self::part_size_for(size);`
2. Compute the optional state file: `let state_file = self.upload_state_file(key);`
   (returns `Option<File>` — `None` when `upload_state_dir` is `None`).
3. Resolve a starting point:
   ```
   let start = match &state_file {
       Some(f) => self.resume_or_restart(f, key, size, part_size).await?,
       None => None,
   };
   ```
   `resume_or_restart` returns `Option<ResumeState>` where
   `struct ResumeState { upload_id: String, completed_parts: Vec<CompletedPart>, offset: u64, next_part_number: i32 }`.
   Behavior (see helper below).
4. If `start` is `None` (fresh): `create_multipart_upload` → extract `upload_id`
   (existing `InvalidResponseErr` on missing id) → **persist state** (if
   `state_file` is `Some`, `write_json(&UploadState{...}, WriteOptions::OVERWRITE_ATOMIC)`)
   → seed `completed_parts = vec![]`, `offset = 0`, `part_number = 1`.
   If `start` is `Some(rs)`: reuse `rs.upload_id`, `rs.completed_parts`, `rs.offset`,
   `rs.next_part_number`.
5. Run `upload_parts_and_complete(key, file, size, part_size, &upload_id, completed_parts, offset, part_number)`.
6. On `Ok(())`: delete the state file (if `Some`, best-effort `file.delete().await`),
   return `Ok(())`.
7. On `Err(err)`: best-effort `abort_multipart_upload` (as today) **and** best-effort
   delete the state file, then return `Err(err)`.

**`upload_parts_and_complete`** — change signature to accept the starting
`completed_parts: Vec<CompletedPart>`, `offset: u64`, `part_number: i32` instead of
initializing them to empty/0/1. Loop body unchanged. (The one abort funnel in
`put_multipart` still wraps it.)

**New helper `upload_state_file`**:
```rust
/// The state file for `key`, or `None` when no upload_state_dir is configured.
fn upload_state_file(&self, key: &str) -> Option<File> {
    self.upload_state_dir
        .as_ref()
        .map(|dir| dir.file(&format!("{}.json", sanitize_filename(key))))
}
```
Note collision guard: `sanitize_filename` can map distinct keys to the same name;
the `UploadState.key` field is verified on load (see below), and a mismatch is
treated as "no resumable state" (fresh upload, overwriting the file). (If a stable
hash helper already exists in `crate::crypt`, the implementer may key the filename
by hash instead; sanitize + in-file key check is the acceptable default.)

**New helper `resume_or_restart`** (the heart of the feature):
```
async fn resume_or_restart(&self, state_file: &File, key: &str, size: u64, part_size: u64)
    -> Result<Option<ResumeState>, S3Err>
```
- If `!state_file.exists()` → `Ok(None)`.
- Read `UploadState` via `read_json` (a corrupt/unreadable file ⇒ treat as no
  state: best-effort delete + `Ok(None)`; do not hard-fail an upload on a bad
  local hint).
- If `state.key != key || state.size != size || state.part_size != part_size`
  (changed source file / incompatible): best-effort `abort_multipart_upload(state.upload_id)`
  + delete state file → `Ok(None)` (caller starts fresh).
- Else call `list_parts(upload_id)` (with pagination, see below):
  - On a **404 / NoSuchUpload** (same raw_response status-404 detection used by
    `get`/`exists`): the upload expired or was lifecycle-aborted → delete state
    file → `Ok(None)`.
  - On any other SDK error → propagate via `map_sdk_err_common("list_parts", ...)`.
  - On success → build the **contiguous prefix**: sort/scan parts by part number;
    take the longest run 1,2,…,N where every part is present, each `e_tag` is
    `Some`, and each part's `size` equals `part_size` (interior parts are always
    full-size; a wrong size ⇒ stop the run). Build
    `completed_parts` = `CompletedPart{ part_number, e_tag }` for 1..=N,
    `offset = N * part_size` (capped at `size`), `next_part_number = N+1`.
    Return `Ok(Some(ResumeState{ upload_id, completed_parts, offset, next_part_number }))`.
    (N may be 0 ⇒ resume the same upload_id from the start.)

**`list_parts` pagination**: loop issuing `list_parts()...part_number_marker(...)`
until `is_truncated()` is not `true`, accumulating `Part`s. Up to 10,000 parts ⇒
up to ~10 pages.

**Doc fixes while here** (stale after earlier renames): module doc line ~7
`[`S3Store`]` → `[`Store`]`; lines ~137 and ~210 `[`Self::put_object_multipart`]`
→ `[`Self::put_multipart`]`; module doc line ~15 references the removed
`DEFAULT_MULTIPART_THRESHOLD` — reword to `Options.part_size`. Add a short module-doc
paragraph documenting resumable multipart + that it requires `upload_state_dir`.

### `agent/src/s3/errors.rs`
Likely **no new variants needed**: filesystem errors already convert via the
existing `From<FileSysErr> for S3Err`; malformed S3 responses reuse
`InvalidResponseErr`; SDK/network errors reuse `map_sdk_err_common`. Only add a
variant if a genuinely new failure mode appears during implementation.

### `agent/tests/s3/mod.rs`
- Update the `store_with`/`store_with_part_size` helper `Config` literal to add
  `upload_state_dir: None` (keeps all existing tests behavior-identical).
- Update the `construction::new_builds_without_network` `Config` literal to add
  `upload_state_dir: None`.
- Add a resumable-upload helper, e.g.
  `resumable_store(events, part_size, state_dir: &Dir) -> (Store, StaticReplayClient)`
  that builds `Config { …, upload_state_dir: Some(Dir::new(state_dir.path())) }`.
- Tests own their state dir via `tempfile::TempDir` (kept alive in the test body);
  `use miru_agent::filesys::dir::Dir;`.

## Test steps (new `pub mod resumable`)

Use a **16 MiB fixture** (`temp_file_with` a 16 MiB buffer) so `part_size_for` ⇒
two 8 MiB parts. Pre-seed the state dir by writing an `UploadState` JSON at
`<dir>/<sanitize(key)>.json` (tests can write the JSON via `serde_json` + `std::fs`,
or via `File::write_json`) with a canned `upload_id`.

1. **Resume skips landed parts.** State pre-seeded (size = 16 MiB, part_size = 8 MiB).
   Replay: `list_parts` → returns part 1 (size 8 MiB, etag `"etag-1"`) →
   `upload_part` for part 2 → `complete`. Assert: exactly one `upload_part` (part 2)
   fired, `complete`’s body includes both etags, and the state file is deleted afterward.
2. **Expired upload_id restarts.** State pre-seeded. Replay: `list_parts` → 404
   (NoSuchUpload) → `create_multipart_upload` (new id) → both `upload_part`s →
   `complete`. Assert create fired after the 404 and the state file is deleted.
3. **Changed source file restarts.** State pre-seeded with `size` ≠ current file
   size. Replay: `abort_multipart_upload`(old id) → `create` → parts → `complete`.
   Assert abort fired before create; state file deleted.
4. **Fresh upload persists then cleans up.** No pre-seeded state, `upload_state_dir:
   Some`. Replay: `create` → parts → `complete`. Assert the state file existed
   after `create` is not observable via replay, but *is* gone after success (assert
   `!state_file.exists()`), and (optional) that on a forced mid-way failure the
   state file is removed and an `abort` fired.
5. **No state dir ⇒ unchanged behavior.** With `upload_state_dir: None`, the
   existing multipart tests still pass (create → parts → complete, abort on error),
   with no `list_parts` call.
6. Keep the 3 `part_size_for` unit tests unchanged and passing.

Also keep all existing s3 tests green (put/get/delete/exists happy + failure paths,
the abort tests, construction, error mapping).

## Validation

- Build: `cargo build --features test -p miru-agent`.
- Tests: `./scripts/test.sh` (`RUST_LOG=off cargo test --features test`) — all pass.
- Lint: `scripts/lint.sh` (import linter, `cargo fmt --check`, machete/diet, audit,
  clippy `-D warnings`; also watch for broken intra-doc links after the doc fixes).
- Coverage: `scripts/covgate.sh` — the `agent/src/s3/.covgate` gate (88.00%) must
  still pass; the new resume paths must be covered by the tests above. (The
  unrelated `workers` covgate shortfall pre-exists on `main` and is out of scope.)
- **Preflight must report `clean` before the changes are pushed.**
