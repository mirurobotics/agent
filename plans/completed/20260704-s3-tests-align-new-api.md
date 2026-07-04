# ExecPlan: Align s3 tests with the reworked s3 API

## Goal

The user reworked the s3 module API in `agent/src/s3/mod.rs`. The test file
`agent/tests/s3/mod.rs` is ~90% updated already but does **not compile** — finish
it so `agent/tests/s3/mod.rs` matches the new API. Scope is the test file only
(no external `src` callers exist). Formatting normalization of the touched files
is in scope.

## New API (already in `agent/src/s3/mod.rs` — do not change it)

- Struct `S3Store` → **`Store`**.
- New `pub struct Config { creds: Credentials, region: String, bucket: String }`.
- New `pub struct Options { part_size: u64 }` with `Default` (8 MiB).
- `Credentials` now has a `Default` impl (dummy test creds).
- Constructors:
  - `Store::new(cfg: Config, opts: Options)` (was `new(creds, region, bucket)`).
  - `#[cfg(feature = "test")] Store::from_http_client(http_client, cfg: Config, opts: Options)` (was `with_http_client(http_client, region, bucket)`).
- **`set_single_put_threshold` was removed.** Multipart is now forced by
  constructing the `Store` with `Options { part_size: 0 }` (the branch is
  `if size > self.opts.part_size { multipart } else { singlepart }`). The actual
  multipart chunk size still comes from the `PART_SIZE` const via `part_size_for`,
  so a tiny fixture with `part_size: 0` still uploads as a single part — identical
  request sequence to the old `set_single_put_threshold(0)` behavior.
- Method renames: `put_file`→`put`, `get_object`→`get`, `delete_object`→`delete`,
  `object_exists`→`exists`. (These call-site renames are already done in the test file.)

## What still breaks compilation (fix these)

### 1. Six `store.set_single_put_threshold(0)` calls (removed method)
In `agent/tests/s3/mod.rs`, these multipart tests call the deleted setter:
`large_file_uploads_in_parts`, `create_missing_upload_id_is_invalid_response`,
`create_failure_maps_to_request_failed`, `upload_part_failure_aborts`,
`part_failure_aborts_upload` (i.e. the ~5–6 tests under `pub mod put` that use
`store_with(...)` then `store.set_single_put_threshold(0)`).

Fix: add a helper next to `store_with`:
```rust
/// Wires a `Store` whose multipart threshold is `part_size`, so multipart tests
/// force the multipart path with tiny fixtures (`part_size: 0` → always multipart).
fn store_with_part_size(events: Vec<ReplayEvent>, part_size: u64) -> (Store, StaticReplayClient) {
    let replay = StaticReplayClient::new(events);
    let cfg = Config {
        region: REGION.to_string(),
        bucket: BUCKET.to_string(),
        creds: Credentials::default(),
    };
    let store = Store::from_http_client(replay.clone(), cfg, Options { part_size });
    (store, replay)
}
```
Refactor `store_with` to delegate: `store_with(events)` = `store_with_part_size(events, Options::default().part_size)`.

Then in each of the 6 tests:
- Replace `let (mut store, replay) = store_with(vec![...]);` + `store.set_single_put_threshold(0);`
  with `let (store, replay) = store_with_part_size(vec![...], 0);`
  (drop the now-unneeded `mut` and the setter line — leftover `mut` will trip clippy's `unused_mut`).
- Some of these bind `_replay` instead of `replay`; preserve whichever binding the test already uses.

### 2. `construction::new_builds_without_network` — old 3-arg `Store::new`
Currently:
```rust
let creds = Credentials { access_key_id: ..., secret_access_key: ..., session_token: ... };
let _store = Store::new(creds, "us-west-2".to_string(), "prod-bucket".to_string());
```
Fix to the new signature:
```rust
let cfg = Config {
    creds: Credentials { access_key_id: "AKIA_TEST".to_string(), secret_access_key: "secret".to_string(), session_token: "session".to_string() },
    region: "us-west-2".to_string(),
    bucket: "prod-bucket".to_string(),
};
let _store = Store::new(cfg, Options::default());
```

### 3. Imports + formatting
- The test file now has two separate `use miru_agent::s3::{...}` lines
  (`{Config, Options, Credentials}` and `{S3Err, Store}`). Merge them into one
  alphabetized import: `use miru_agent::s3::{Config, Credentials, Options, S3Err, Store};`
  (keep the `use miru_agent::s3::errors::{...}` line separate — it is a submodule path).
- Update the stale comment in `large_file_uploads_in_parts` that says "default
  256 MiB part size" → the part size is 8 MiB now; reword to match (e.g. "the
  8 MiB part size dwarfs the file"). Also drop/soften the "threshold set to 0
  below" phrasing so it refers to `store_with_part_size(.., 0)`.
- Run `cargo fmt -p miru-agent`. NOTE: the user's `src/s3/mod.rs` change contains
  trailing whitespace (e.g. after the `Options` `Default` impl) that will fail
  `cargo fmt --check`. Running fmt on the package will normalize it — this is a
  whitespace-only touch to `src`, acceptable and expected to keep preflight green.

## Test steps

1. `put_streams_file_body_bytes` (single-PUT) passes via `store_with`.
2. All 6 multipart tests pass via `store_with_part_size(.., 0)` (create → upload_part → complete sequence, and the abort-on-failure paths) — request assertions unchanged.
3. `put_missing_source_maps_to_filesys_err` still expects `S3Err::FileSysErr(_)`.
4. `get`/`delete`/`exists` happy-path and `request_failed` tests pass with renamed methods.
5. `construction::new_builds_without_network` compiles and passes with `Store::new(Config, Options)`.
6. The 3 `part_size_for` unit tests in `src` (already referencing `Store::`) pass.

## Validation

- Build: `cargo build --features test -p miru-agent`.
- Tests: `./scripts/test.sh` (`RUST_LOG=off cargo test --features test`) — all pass.
- Lint: `scripts/lint.sh` (import linter, `cargo fmt --check`, machete/diet, audit, clippy `-D warnings`).
- Coverage: `scripts/covgate.sh` — the `agent/src/s3/.covgate` gate (88.00%) must pass. (Note: an unrelated `workers` covgate shortfall pre-exists on `main` and is out of scope.)
- **Preflight must report `clean` before the changes are pushed.**
