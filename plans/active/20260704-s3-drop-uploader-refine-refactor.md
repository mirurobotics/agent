# ExecPlan: Drop s3 uploader; land + refine the Store API rework

## Context

The user reworked `agent/src/s3/mod.rs` (uncommitted) so the `Store` client is
bucket-agnostic and options are per-call:

- `Store` no longer stores `bucket` or `opts`. `Config` is now `{ creds, region }`
  (no `bucket`).
- New `pub struct Object { bucket, key }` (Display `s3://{bucket}/{key}`) carries
  the target per operation. New `PutOptions { part_size }` (was `Options`), passed
  to `put`. New `PartToUpload { upload_id, number, offset, length }` and
  `UploadedPart { number, etag, size }` (the latter replaces the removed `PartInfo`
  and is used by both `list_parts` and `complete_multipart_upload`).
- Every method takes `&Object` instead of `key: &str`: `put(src, dst, opts)`,
  `put_singlepart(src, dst)` (renamed from `put_object`), `put_multipart(src, dst, size)`,
  `create_multipart_upload(dst)`, `upload_part(src, dst, &PartToUpload)`,
  `complete_multipart_upload(obj, upload_id, &[UploadedPart])`,
  `abort_multipart_upload(obj, upload_id)`, `list_parts(obj, upload_id) -> Option<Vec<UploadedPart>>`,
  `get(src, dest)`, `delete(obj)`, `exists(obj)`.
- `multipart_threshold()` accessor removed (opts no longer on the Store).

The tree does not compile: the `uploader` module and all tests still use the old
API, and `mod.rs` still has `uploader` doc-refs.

The user wants: **drop the `uploader` module** (re-added later), **commit the
refactor**, then **review + refine** it and **adjust tests**. Deliver as two commits.

## Commit 1 — drop uploader + land the refactor (make it green)

Goal: the user's `mod.rs` logic stays **as-authored** (do not refine it here);
only remove `uploader` references and migrate the tests so the tree builds and
passes.

1. **Delete** `agent/src/s3/uploader.rs`.
2. In `agent/src/s3/mod.rs`: remove `pub mod uploader;` (line ~38); remove the
   module-doc sentence referencing `[`uploader::Uploader`]` (line ~22); in the
   `put_multipart` doc (line ~196) drop the "for resumable uploads use
   `[`uploader::Uploader`]`" clause — reword to note this variant is stateless and
   a resumable variant will return later. Do **not** otherwise change `mod.rs` logic.
3. **Rewrite `agent/tests/s3/mod.rs` to the new API:**
   - Remove `use miru_agent::s3::uploader::Uploader;` and `use ...::filesys::dir::Dir;`
     (if only used by uploader tests). Import `Object`, `PutOptions`.
   - `Config` literal: drop `bucket` (and any `upload_state_dir`) → `{ creds, region }`.
   - `Store::from_http_client(replay, cfg)` and `Store::new(cfg)` — no `opts` arg.
   - Replace the `store_with_part_size(events, part_size)` helper: since part size
     is now per-call, keep a single `store_with(events) -> (Store, StaticReplayClient)`
     and pass `PutOptions { part_size }` at each `put` call site. A small
     `obj(key) -> Object` helper building `Object { bucket: BUCKET.into(), key: key.into() }`
     keeps call sites terse.
   - Migrate every call: `store.put(&File::new(src.path()), &obj(key), PutOptions::default())`
     (single) / `PutOptions { part_size: 0 }` (force multipart);
     `store.get(&obj(key), &File::new(dest.path()))`; `store.delete(&obj(key))`;
     `store.exists(&obj(key))`; `store.put_singlepart(&File::new(...), &obj(key))`;
     the `put_source_missing` test → `S3Err::FileSysErr(_)` unchanged.
   - **Remove the `pub mod resumable` uploader tests** entirely (the `uploader_with`
     helper, `seed_state`, and the 6 resume/`store()` tests) — they go with the module.
   - Keep the 3 `part_size_for` unit tests (in `mod.rs`, referencing `Store::part_size_for`).

Validate: `cargo build --features test -p miru-agent` + `RUST_LOG=off cargo test
--features test -p miru-agent s3` green; `cargo fmt`. → orchestrator commits as
`refactor(agent): rework s3 Store API around Object/PutOptions; drop uploader`.

## Commit 2 — review + refine + add coverage tests

Review the landed refactor and apply conservative refinements (the design is the
user's; keep it — polish only):

1. **`upload_part` shadowing:** the response binding `let part = self.client
   .upload_part()...` shadows the `part: &PartToUpload` parameter. Rename the
   response binding (e.g. `output`/`resp`) for readability.
2. **Stale/ misplaced doc:** the "Split out from `put_multipart` so a single `?`
   early-return funnels through one abort site" comment now sits on `upload_parts`
   but describes the `upload_parts_and_complete`/`put_multipart` abort funnel.
   Re-point it: document `upload_parts` for what it does (uploads all parts, returns
   `Vec<UploadedPart>`), and keep the abort-funnel rationale on
   `upload_parts_and_complete`.
3. **Helper consistency:** `map_bytestream_err` now takes `&Object` but
   `map_body_io_err` still takes `key: &str` (called from `get` with `&src.key`).
   Make `map_body_io_err` take `&Object` too and pass `src`, for a consistent
   `s3://…`-formatted message. (`get` is its only caller — 2 call sites.)
4. **Derives:** add `#[derive(Debug, Clone)]` to `Object`, `PartToUpload`,
   `UploadedPart`, `PutOptions`, and `#[derive(PartialEq, Eq)]` to `Object` and
   `UploadedPart` where it aids test assertions. Conservative — no behavior change.
5. Watch for any other reviewer-grade nits (unnecessary clones, dead code, doc
   accuracy) but do not redesign the API.

**Add/adjust tests** (the primitives are now public building blocks with the
`uploader` — their only in-tree consumer — removed):
- `list_parts` has no remaining caller, so it is otherwise uncovered. Add direct
  tests: happy path (returns `UploadedPart`s), **pagination** (two pages via
  `is_truncated` / `next_part_number_marker`), and **404 → `Ok(None)`**. Model the
  canned `ListPartsResult` XML on the existing multipart-response helpers.
- Confirm `create_multipart_upload` / `upload_part` / `complete_multipart_upload` /
  `abort_multipart_upload` stay covered via the migrated `put_multipart` + abort
  tests; add a focused primitive test only where coverage requires.

Validate green + fmt. → orchestrator commits as
`refactor(agent): refine s3 Store API after review; cover multipart primitives`.

## Validation (whole task)

- Build: `cargo build --features test -p miru-agent`.
- Tests: `./scripts/test.sh` — all pass.
- Lint: `scripts/lint.sh` (import linter, `cargo fmt --check`, machete/diet, audit,
  clippy `-D warnings`; verify no broken intra-doc links after removing `uploader` refs).
- Coverage: `scripts/covgate.sh` — `agent/src/s3/.covgate` (88.00%) must pass. Removing
  the uploader deletes both code and its tests; the migrated + new primitive tests must
  keep the gate green. (The `workers` covgate shortfall pre-exists on `main` and is out
  of scope.)
- **Preflight must report `clean` before the changes are pushed.**
