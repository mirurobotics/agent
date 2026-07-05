# ExecPlan: `Store::resume_multipart_upload` + finish the multipart-API refactor

## Context

The user is mid-refactor of `agent/src/s3/` (uncommitted). Their goal: add a
`resume_multipart_upload` method and simplify the multipart API. The working tree
**does not compile** and several tests are broken against the new API. This task
completes the refactor and lands it green.

### The user's refactor (already in the tree — keep it, build on it)
- `put(src: File, dst: &Object)` — takes `File` by value, no `PutOptions`; the
  single-vs-multipart threshold is the fixed `PART_SIZE` (8 MiB) const in `mod.rs`.
- `put_multipart(src: &multipart::Source, dst: &Object)` where
  `pub struct Source { pub file: File, pub size: u64 }`.
- `upload_part`, `complete_multipart_upload`, `list_parts`, `list_parts_page`,
  `upload_parts` are now **private**. Public multipart surface: `put_multipart`,
  `create_multipart_upload`, `exec_multipart_upload(&Source, &Object, upload_id)`,
  `abort_multipart_upload`, and (new) `resume_multipart_upload`.
- `PutOptions` was deleted. `UploadedPart { number, etag }` (no `size`).
- An **empty** `resume_multipart_upload` stub exists at `multipart.rs:130`.

## 1. Implement `resume_multipart_upload` (`agent/src/s3/multipart.rs`)

Fill the stub with exactly this signature (the user's — takes `&File`, computes
size internally; note the minor asymmetry with `put_multipart`'s `&Source`, which
is intentional per the user):

```rust
pub async fn resume_multipart_upload(
    &self,
    src: &File,
    dst: &Object,
    upload_id: &str,
) -> Result<(), S3Err>
```

Behavior:
1. `let size = src.size().await?;` (`FileSysErr` converts to `S3Err` via the
   existing `From`). `let part_size = Self::part_size_for(size);`
2. Discover landed parts: `let landed = self.list_parts(dst, upload_id).await?;`.
   - `Ok(None)` (S3 no longer knows the upload → 404/NoSuchUpload) → the upload
     expired or was aborted; **cannot resume** → return `S3Err::NoSuchUploadErr`
     (new variant, see §2) carrying `dst.key` + `upload_id`.
   - `Ok(Some(parts))` → index by part number: `HashMap<i32, String>` (number → etag).
3. **Gap-fill loop** over the full range. `total_parts = size.div_ceil(part_size)`
   (guard `size == 0` → treat as no parts). For `n in 1..=total_parts`:
   - `offset = (n-1) * part_size`, `len = part_size.min(size - offset)`.
   - if `n` is in the landed map → reuse its etag (`UploadedPart { number: n, etag }`).
   - else → `upload_part(src, dst, &PartToUpload { upload_id, number: n, offset, length: len })`
     and use the returned etag.
   - push the `UploadedPart` (in ascending order).
4. `self.complete_multipart_upload(dst, upload_id, &parts).await`.

**Do NOT abort on error.** Unlike `put_multipart`, resume must be safely
re-runnable: on any failure just propagate the error and leave the upload intact so
a later call can resume again. Document this and the precondition: `src` must be the
**same file** that the original upload was for (same bytes ⇒ same size ⇒ same
`part_size` ⇒ part `n` maps to the identical byte range; landed ETags are trusted,
not re-hashed). Add a thorough doc comment.

Import `std::collections::HashMap` (internal-crates group).

## 2. Add `NoSuchUploadErr` (`agent/src/s3/errors.rs`)

Mirror `ObjectNotFoundErr`:
```rust
#[derive(Debug, thiserror::Error)]
#[error("no such multipart upload '{upload_id}' for object '{key}'")]
pub struct NoSuchUploadErr { pub key: String, pub upload_id: String, pub trace: Box<Trace> }
impl crate::errors::Error for NoSuchUploadErr {
    fn code(&self) -> Code { Code::ResourceNotFound }
    fn http_status(&self) -> HTTPCode { HTTPCode::NOT_FOUND }
}
```
Add the `NoSuchUploadErr(NoSuchUploadErr)` variant to the `S3Err` enum and to the
`crate::impl_error!(S3Err { ... })` list. Import it in `multipart.rs`.

## 3. Fix the offline tests (`agent/tests/s3/mod.rs`, `agent/tests/s3/multipart.rs`)

The refactor removed `PutOptions` and privatised `list_parts`/`upload_part`/
`complete_multipart_upload`, so:
- **`tests/s3/mod.rs`**: drop `PutOptions` from the import. Migrate the `put`
  call sites: single-part stays `put_singlepart(&File::new(..), &obj(key))`; the
  `put(.., PutOptions::default())` sites become `put(File::new(..), &obj(key))`
  (note: `put` now takes `File` by value). The `put_source_missing` test →
  `put(File::new(missing), &obj("k"))`, still expecting `S3Err::FileSysErr(_)`.
- **`tests/s3/multipart.rs`** is currently **orphaned** (its `pub mod multipart;`
  was removed from `tests/s3/mod.rs`). Re-wire it: add `pub mod multipart;` back to
  `tests/s3/mod.rs`. Then migrate it:
  - Multipart `put` tests: replace `store.put(&File, &obj, PutOptions { part_size: 0 })`
    with `store.put_multipart(&Source { file: File::new(src.path()), size: <byte len> }, &obj(key))`
    (import `miru_agent::s3::multipart::Source`). A tiny fixture ⇒ `part_size_for`
    is 8 MiB ⇒ a single part ⇒ the same create→upload_part→complete replay sequence.
    Keep the abort-on-failure tests (drive them the same way).
  - **Remove the three direct `list_parts` tests** (`list_parts` is now private and
    unreachable from the test crate) — their coverage moves to the resume tests below.
- Add a **`pub mod resume`** submodule in `tests/s3/multipart.rs` with
  `StaticReplayClient` tests for `resume_multipart_upload` (canned `ListParts` +
  `UploadPart` + `Complete` XML, modelled on the existing multipart helpers):
  1. **resume uploads only missing parts** — a 2-part upload (use a ~in-replay
     scenario where `list_parts` returns part 1) → assert only part 2 is uploaded,
     then `complete` carries both etags.
  2. **resume with no landed parts** — `list_parts` returns empty → both parts
     uploaded → complete.
  3. **resume with all parts landed** — `list_parts` returns both → no `upload_part`
     fires → complete directly.
  4. **expired upload → `NoSuchUploadErr`** — `list_parts` 404 → assert
     `Err(S3Err::NoSuchUploadErr(_))`, and assert no `upload_part`/`complete` fired.
  5. **pagination** — `list_parts` truncated across two pages feeds the landed set.
  These tests are what now cover `list_parts` / `list_parts_page` for the covgate.
  Multi-part scenarios need a source file larger than `PART_SIZE`; use a helper that
  writes an 8-MiB-plus fixture (as the earlier resume tests did) so `part_size_for`
  yields ≥2 parts, OR structure the replay so the byte ranges line up — the
  implementer picks whichever keeps the replay assertions honest.

## 4. Fix the integration tests (`agent/tests/s3_integration.rs`)

They currently use `PutOptions` and directly call the now-private
`upload_part`/`list_parts`/`complete_multipart_upload`. Migrate to the **public**
API so the target compiles:
- drop `PutOptions`, `PartToUpload`, `UploadedPart` from the import if only the
  removed calls used them.
- `put` call → `put(File::new(..), &obj)` (by value, no options).
- Replace the `multipart_primitives` test (private primitives) with a
  `put_multipart` round-trip using `put_multipart(&Source { file, size }, &obj)`,
  plus a new **`resume_round_trip`** integration test: `create_multipart_upload` →
  `resume_multipart_upload` (from byte 0, no parts landed yet, so it uploads all
  parts and completes) → `get` → verify bytes → `delete`. Keep the gated/skip-without-creds
  structure intact.
- Keep `single_part_round_trip`, `abort_discards_upload` (uses public
  `create`/`abort`), and `get_missing_key_is_not_found`, migrated to the new `put`.

## Test steps
1. Offline: `resume` submodule — uploads only missing parts / none-landed /
   all-landed / `NoSuchUploadErr` on 404 / pagination.
2. Offline: migrated `put_multipart` create→part→complete + abort tests pass.
3. Offline: `put`/`put_singlepart`/`get`/`delete`/`exists` + error-mapping tests pass.
4. The 3 `part_size_for` unit tests pass unchanged.
5. Integration target compiles and all tests skip cleanly without creds.

## Validation
- Build: `cargo build --features test -p miru-agent`.
- Tests: `./scripts/test.sh` — all pass; `cargo test --features test --test s3_integration`
  compiles and skips.
- Lint: `scripts/lint.sh` (import linter, fmt, machete/diet, audit, clippy `-D warnings`;
  no broken intra-doc links).
- Coverage: `scripts/covgate.sh` — `agent/src/s3/.covgate` (88.00%) must pass; the
  new resume tests must cover `resume_multipart_upload` + `list_parts`/`list_parts_page`.
  (`workers` covgate shortfall pre-exists on `main`; out of scope.)
- **Preflight must report `clean` before the changes are pushed.**
