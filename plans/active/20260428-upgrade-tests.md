# Tests for `app::upgrade` (`needs_upgrade`, `reconcile_impl`, `reconcile`)

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | All test additions and the deletion of one stale test live in this repo. |

All changes are confined to `agent/agent/tests/app/upgrade.rs` (the `miru-agent` Rust crate at `agent/agent/`).

## Purpose / Big Picture

`agent/src/app/upgrade.rs` performs an idempotent rebootstrap of on-disk state when the running agent version differs from the marker on disk. Public entry points: `needs_upgrade`, `reconcile_impl`, `reconcile`.

The existing `reconcile_returns_uninstalled_err_when_no_device_id_resolvable` test no longer compiles against the current `reconcile_impl` signature, so the entire `tests/app/upgrade.rs` file fails to build. After M1 the file compiles and the 4 pre-existing `reconcile_*` tests pass; M2–M4 add 10 new tests, bringing the post-completion total to 14 passing tests.

## Progress

- [ ] M1: Delete the broken `reconcile_returns_uninstalled_err_when_no_device_id_resolvable` test so `tests/app/upgrade.rs` compiles, then confirm `cargo test` runs.
- [ ] M2: Add four `needs_upgrade` tests (missing marker, match, mismatch, read-error).
- [ ] M3: Add five `reconcile_impl` tests (happy path + one representative failure per pipeline step: FileSysErr from key check, HTTPErr from get_device, StorageErr from reset, HTTPErr from update_device).
- [ ] M4: Verify and (if missing) add one `reconcile` test that asserts the backoff loop is exercised at least N times before final success, using a counting no-op `sleep_fn`.
- [ ] Final: preflight clean (formatting, clippy, tests).

Use timestamps when you complete steps.

## Surprises & Discoveries

(Add entries as you go.)

## Decision Log

- Decision: Place all new tests in the existing integration test file `agent/agent/tests/app/upgrade.rs` rather than an inline `#[cfg(test)] mod` inside `agent/src/app/upgrade.rs`.
  Rationale: The five existing upgrade tests already live in `tests/app/upgrade.rs` with a working harness (`prepare_layout`, `make_mock_client`, `backend_device`, `read_keys`). Reusing it is simpler than duplicating helpers inline, and keeps coverage discoverable. The "read-error" case for `needs_upgrade` does **not** require module-private access — it can be triggered by making `layout.agent_version()` return a path that exists but contains invalid content (e.g. write a directory in its place, or write bytes that fail the storage decode), so an integration test is sufficient.
  Date/Author: 2026-04-28 / plan author.

- Decision: Do not introduce any new `pub` items in `agent/src/app/upgrade.rs` for testing. The private helpers `issue_token`, `fetch_device`, `update_device` are exercised indirectly through `reconcile_impl`. `reconcile_impl` is already `pub` in the source as of today (verified by inspecting the `reconcile_impl` declaration in `agent/src/app/upgrade.rs`).
  Rationale: Avoid widening public surface for tests; the integration tests in `agent/agent/tests/` consume `miru_agent::app::upgrade::*` as a downstream crate.
  Date/Author: 2026-04-28 / plan author.

- Decision: For `reconcile` retry tests, inject `|_| async {}` (a no-op sleep) instead of `tokio::time::sleep`. The three pre-existing `reconcile_*` tests use the real `tokio::time::sleep` because the first retry only waits 1 second; that is fine and we keep it. New retry tests that need many iterations use the no-op sleep to avoid adding wall-clock latency.
  Rationale: Per task constraints — never use real sleep in tests for `reconcile` unless the existing tests already do.
  Date/Author: 2026-04-28 / plan author.

- Decision: Delete `reconcile_returns_uninstalled_err_when_no_device_id_resolvable` outright; do not treat any new test as its replacement.
  Rationale: The old test premise — that `reconcile` returns `UpgradeErr::StorageErr(StorageErr::ResolveDeviceIDErr(_))` when no device id is on disk — is dead because `reconcile_impl` no longer calls `storage::resolve_device_id`; the device is fetched from the backend instead. The new `reconcile_impl_returns_storage_err_when_reset_fails` test added under M3 is independent coverage of the storage-failure path inside `reconcile_impl`, not a "replacement" — `reconcile_impl` returning `UpgradeErr::StorageErr` via the `#[from]` conversion is a property worth asserting on its own merits.
  Date/Author: 2026-04-28 / plan author.

- Decision: Do not pass `--features test`. `agent/agent/Cargo.toml` declares `[features] test = []` — an empty feature list. `--features test` is only required when a test references a `#[cfg(feature = "test")]` symbol, and none of the new tests do. The canonical invocation for this work is `cargo test -p miru-agent --test mod`.
  Date/Author: 2026-04-28 / plan author.

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

Crate: `miru-agent` at `agent/agent/` (manifest `agent/agent/Cargo.toml`).

**Code under test — `agent/agent/src/app/upgrade.rs`:**

- `pub async fn needs_upgrade(layout: &Layout, cur_version: &str) -> bool`. Reads `layout.agent_version()` via `storage::agent_version::read`. Returns `true` when the marker is missing, mismatched, or the read errors (logged and treated as missing); `false` only when the marker matches `cur_version`.

- `pub async fn reconcile_impl<HTTPClientT: ClientI>(http_client, layout, version) -> Result<(), UpgradeErr>`. Pipeline:
  1. `issue_token` — asserts `auth/private_key` and `auth/public_key` exist (`FileSysErr` on missing key file), then calls `authn::issue_token` (`AuthnErr` / `HTTPErr` on JWT or HTTP failure).
  2. `fetch_device` — `http::devices::get(&token)` (`HTTPErr` on failure).
  3. `storage::setup::reset(layout, &device, &Settings::default(), version)` — writes `auth/`, `device.json`, `settings.json`, blank token, wipes `resources/`, writes the agent-version marker LAST (`StorageErr` on failure).
  4. `update_device` — `http::devices::update` with the running version (`HTTPErr` on failure). Runs **after** `setup::reset`, so a failure here leaves the marker on disk.

- `pub async fn reconcile<F, Fut, HTTPClientT: ClientI>(layout, http_client, version, sleep_fn)`. Loop: return early when `!needs_upgrade(...)`; otherwise call `reconcile_impl`, on `Err` log, `err_streak += 1`, sleep via `sleep_fn` for `cooldown::calc(&Backoff { base_secs: 1, growth_factor: 2, max_secs: 60 }, err_streak)` seconds, retry. **Retries on every error type, not just network** — tests must arrange eventual success or they hang.

**Errors — `agent/agent/src/app/errors.rs`:**

    pub enum UpgradeErr {
        StorageErr(#[from] storage::StorageErr),
        HTTPErr(#[from] http::HTTPErr),
        AuthnErr(#[from] authn::AuthnErr),
        FileSysErr(#[from] filesys::FileSysErr),
    }

**Existing test scaffolding — `agent/agent/tests/app/upgrade.rs`:**

- `prepare_layout(name, device_id) -> (Layout, filesys::Dir)` — creates a temp dir, generates a real RSA keypair under `auth/`, writes `device.json` with `device_id`. Hold the `filesys::Dir` for the dir lifetime.
- `make_mock_client(device) -> Arc<MockClient>` — pre-wires `issue_device_token_fn` to a 5-minute-future RFC3339 token and `get_device_fn` to return `device`.
- `backend_device(id, name) -> backend_client::Device`, `read_keys(layout) -> (String, String)`.

**MockClient — `agent/agent/tests/mocks/http_client.rs`:**

- `MockClient::default()` plus struct-update field overrides.
- Runtime setters: `set_get_device(F)`, `set_update_device(F)`, plus deployment/config/release setters.
- Counters: `num_get_device_calls()`, `num_update_device_calls()`, `call_count(Call::IssueDeviceToken)`.
- Failure injection: closures may return `Err(HTTPErr::MockErr(HTTPMockErr { is_network_conn_err: true }))`.
- `issue_device_token_fn` is a plain `Box<dyn Fn>` (no setter); to override, construct `MockClient` directly rather than from `make_mock_client`.

**Storage — `agent/agent/src/storage/`:**

- `storage::agent_version::read(file) -> Result<Option<String>, StorageErr>` (returns `Ok(None)` when missing).
- `storage::agent_version::write(file, version) -> Result<(), StorageErr>` (for seeding markers).
- `storage::setup::reset(layout, &Device, &Settings, version) -> Result<(), StorageErr>`.

**`Layout` — `agent/agent/src/storage/layout.rs`:** `layout.agent_version()` returns a `filesys::File` for the marker; `layout.auth().private_key()` / `.public_key()` return key file wrappers; `layout.device()` returns the `device.json` `filesys::File`. Use `.path()` on a `filesys::File` to get the underlying `&Path` for `tokio::fs` calls.

## Plan of Work

All edits are in `agent/agent/tests/app/upgrade.rs`. No source changes (`reconcile_impl` is already `pub`).

### M1 — Delete the broken test (unblocks compilation)

Delete `reconcile_returns_uninstalled_err_when_no_device_id_resolvable` and its `#[tokio::test]` attribute. After deletion, `cargo test -p miru-agent --test mod -- app::upgrade` should compile and the four pre-existing `reconcile_*` tests pass. Drop newly-unused imports, but keep `UpgradeErr` (re-used by M3).

### M2 — `needs_upgrade` unit tests (4 cases)

Append to `agent/agent/tests/app/upgrade.rs`. Import `miru_agent::app::upgrade::needs_upgrade`. Use `prepare_layout` for the `Layout`.

1. `needs_upgrade_returns_true_when_marker_missing` — no marker; assert `needs_upgrade(&layout, "v1.0.0").await == true`.
2. `needs_upgrade_returns_false_when_marker_matches` — `storage::agent_version::write(&layout.agent_version(), "v1.2.3").await.unwrap()`; assert `false`.
3. `needs_upgrade_returns_true_when_marker_differs` — write `"v1.0.0"`, call with `"v2.0.0"`; expect `true`.
4. `needs_upgrade_returns_true_when_read_errors` — force a read error by creating a directory at the marker path:

        tokio::fs::create_dir_all(layout.agent_version().path()).await.unwrap();

   Mechanism (deterministic): `read` in `agent/src/storage/agent_version.rs` checks `file.exists()` (true for a directory) then `file.read_string().await?`, which fails on a directory and returns `Err(StorageErr::FileSysErr(_))`. Assert `needs_upgrade(...) == true`. If this does not error, record in Surprises & Discoveries.

### M3 — `reconcile_impl` tests (happy path + one representative failure per pipeline step: FileSysErr from key check, HTTPErr from get_device, StorageErr from reset, HTTPErr from update_device)

Import `miru_agent::app::upgrade::reconcile_impl`. Use `prepare_layout` and `make_mock_client` like the existing `reconcile_*` tests.

1. `reconcile_impl_happy_path_writes_marker_and_updates_backend` — assert `Ok(())`, marker on disk equals `version`, `mock.num_update_device_calls() == 1`, `mock.num_get_device_calls() >= 1`, `mock.call_count(Call::IssueDeviceToken) >= 1`.
2. `reconcile_impl_returns_filesys_err_when_private_key_missing` — `tokio::fs::remove_file(&layout.auth().private_key()).await.unwrap()`. Match `Err(UpgradeErr::FileSysErr(_))` (from `private_key_file.assert_exists()?`).
3. `reconcile_impl_returns_http_err_when_get_device_fails` — `mock.set_get_device(|| Err(HTTPErr::MockErr(HTTPMockErr { is_network_conn_err: true })))`. Match `Err(UpgradeErr::HTTPErr(_))`.
4. `reconcile_impl_returns_storage_err_when_reset_fails` — pre-create a **directory** at `layout.device()`. Mechanism (deterministic): `reset` in `agent/src/storage/setup.rs` runs `device_file.write_json(...)` after creating the auth dir; an atomic JSON write cannot replace an existing directory and returns `Err(StorageErr::FileSysErr(_))`. After `prepare_layout`:

        tokio::fs::remove_file(layout.device().path()).await.unwrap();
        tokio::fs::create_dir_all(layout.device().path()).await.unwrap();

   Match `Err(UpgradeErr::StorageErr(_))`. If a different variant surfaces, log it and record in Surprises & Discoveries before adjusting injection.
5. `reconcile_impl_returns_http_err_when_update_device_fails` — happy `get_device`, but `mock.set_update_device(|| Err(HTTPErr::MockErr(HTTPMockErr { is_network_conn_err: true })))`. Match `Err(UpgradeErr::HTTPErr(_))`. Also assert `storage::agent_version::read(&layout.agent_version()).await.unwrap() == Some(version.to_string())` — `setup::reset` already wrote the marker before `update_device` ran.

### M4 — `reconcile` retry-loop tests

Four tests already exist (`reconcile_is_noop_when_marker_matches`, `reconcile_rebootstraps_when_marker_missing`, `reconcile_rebootstraps_when_marker_version_differs`, `reconcile_retries_until_get_device_succeeds`). Keep them. Add one more:

1. `reconcile_uses_injected_sleep_and_recovers_after_repeated_failures` — configure `set_get_device` to fail `N = 4` times then succeed; inject a counting no-op sleep. The file already imports `chrono::Duration`, so add `use std::time::Duration as StdDuration;` and use `StdDuration` in the closure parameter to disambiguate. `reconcile`'s bound is `Fn(Duration) -> impl Future<Output = ()> + Send` (not `FnMut`); `Arc<AtomicUsize>::fetch_add` takes `&self` so the bare `Fn` bound is satisfiable.

        let sleep_count = Arc::new(AtomicUsize::new(0));
        let counter = sleep_count.clone();
        let sleep_fn = move |_: StdDuration| {
            counter.fetch_add(1, Ordering::SeqCst);
            async {}
        };
        reconcile(&layout, mock.as_ref(), "v9.9.9", sleep_fn).await;
        assert_eq!(sleep_count.load(Ordering::SeqCst), 4, "expected exactly 4 sleeps for 4 injected failures");
        assert_eq!(mock.num_update_device_calls(), 1);

## Concrete Steps

### Step 0 — Confirm test harness layout

`agent/agent/tests/mod.rs` is the single integration-test binary; every module under `agent/agent/tests/` (including `app/upgrade.rs`) is compiled into it via `pub mod`. Canonical invocation:

    cargo test -p miru-agent --test mod

No `--features test` (see Decision Log). All `cargo test` lines below use this form; commands run from `agent/agent/` unless noted.

### Step 1 — M1 (delete broken test, establish baseline)

Edit `agent/agent/tests/app/upgrade.rs`. Remove `reconcile_returns_uninstalled_err_when_no_device_id_resolvable` and its `#[tokio::test]` attribute (lines 200–216 pre-edit). Drop newly-unused imports (keep `UpgradeErr`).

    cargo test -p miru-agent --test mod -- app::upgrade

Expected: file compiles; the four pre-existing `reconcile_*` tests pass.

Commit (from `agent/`):

    git add agent/tests/app/upgrade.rs
    git commit -m "test(upgrade): remove obsolete reconcile uninstalled-err test"

### Step 2 — M2 (needs_upgrade tests)

Add `use miru_agent::app::upgrade::needs_upgrade;`. Append the four `needs_upgrade_*` tests from M2.

    cargo test -p miru-agent --test mod -- app::upgrade::needs_upgrade

Expected: 4 new tests pass.

Commit (from `agent/`):

    git add agent/tests/app/upgrade.rs
    git commit -m "test(upgrade): cover needs_upgrade missing/match/mismatch/read-error"

### Step 3 — M3 (reconcile_impl tests)

Add `use miru_agent::app::upgrade::reconcile_impl;`. Append the five `reconcile_impl_*` tests.

    cargo test -p miru-agent --test mod -- app::upgrade::reconcile_impl

Expected: 5 new tests pass.

Commit (from `agent/`):

    git add agent/tests/app/upgrade.rs
    git commit -m "test(upgrade): cover reconcile_impl happy path and per-step failures"

### Step 4 — M4 (reconcile retry test)

Add `std::sync::atomic::{AtomicUsize, Ordering}` and `std::time::Duration as StdDuration` (the alias avoids conflict with the already-imported `chrono::Duration`). Append `reconcile_uses_injected_sleep_and_recovers_after_repeated_failures`.

    cargo test -p miru-agent --test mod -- app::upgrade::reconcile_uses_injected_sleep

Expected: 1 new test passes.

Commit (from `agent/`):

    git add agent/tests/app/upgrade.rs
    git commit -m "test(upgrade): assert reconcile loop sleeps via injected sleep_fn"

### Step 5 — Full preflight

From `agent/`:

    cargo fmt --all -- --check
    cargo clippy -p miru-agent --tests -- -D warnings
    cargo test -p miru-agent --test mod

Expected: all three exit `0`. Record the final passing test count `<N>` in Outcomes & Retrospective.

## Validation and Acceptance

From `agent/agent/`, run:

    cargo test -p miru-agent --test mod -- app::upgrade

and observe all 14 of these test names passing (4 `needs_upgrade` + 5 `reconcile_impl` + 4 existing `reconcile` + 1 new counted-sleep `reconcile`):

- `app::upgrade::needs_upgrade_returns_true_when_marker_missing`
- `app::upgrade::needs_upgrade_returns_false_when_marker_matches`
- `app::upgrade::needs_upgrade_returns_true_when_marker_differs`
- `app::upgrade::needs_upgrade_returns_true_when_read_errors`
- `app::upgrade::reconcile_impl_happy_path_writes_marker_and_updates_backend`
- `app::upgrade::reconcile_impl_returns_filesys_err_when_private_key_missing`
- `app::upgrade::reconcile_impl_returns_http_err_when_get_device_fails`
- `app::upgrade::reconcile_impl_returns_storage_err_when_reset_fails`
- `app::upgrade::reconcile_impl_returns_http_err_when_update_device_fails`
- `app::upgrade::reconcile_is_noop_when_marker_matches` (existing)
- `app::upgrade::reconcile_rebootstraps_when_marker_missing` (existing)
- `app::upgrade::reconcile_rebootstraps_when_marker_version_differs` (existing)
- `app::upgrade::reconcile_retries_until_get_device_succeeds` (existing)
- `app::upgrade::reconcile_uses_injected_sleep_and_recovers_after_repeated_failures`

`reconcile_returns_uninstalled_err_when_no_device_id_resolvable` MUST NOT appear.

**Preflight must report `clean` before changes are published.** From `agent/`:

    cargo fmt --all -- --check
    cargo clippy -p miru-agent --tests -- -D warnings
    cargo test -p miru-agent --test mod

must all exit `0` with no clippy warnings and no failed/ignored tests. Open the PR only after this preflight is clean.

Acceptance: a novice running `cargo test -p miru-agent --test mod` in `agent/agent/` after pulling this branch sees 14 upgrade tests pass; before the branch the file did not compile. `agent/src/app/upgrade.rs` is unchanged — only test files change.

## Idempotence and Recovery

- Every edit to `agent/agent/tests/app/upgrade.rs` is safe to re-run; re-applying is a no-op once the file already contains the new tests, and `cargo test` is naturally idempotent.
- If a test in M2–M4 hangs, the cause is almost certainly a `reconcile` test missing an eventual-success path. Recovery: `Ctrl-C`, locate the test via `cargo test -p miru-agent --test mod -- --nocapture app::upgrade`, and ensure the mock's failure injection has a terminating "now succeed" branch. Do not add `tokio::time::timeout` — fix the mock instead (no-real-sleep constraint).
- If clippy flags an unused import after M1, drop it and re-run.
- Each milestone ends in its own commit (Steps 1–4). If preflight fails, fix forward in a new commit — do not amend, per `repos/agent/CLAUDE.md`.
