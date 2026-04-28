# Tests for `app::upgrade` (`needs_upgrade`, `reconcile_impl`, `reconcile`)

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | All test additions and the deletion of one stale test live in this repo. |

This plan lives in `agent/plans/backlog/` because all changes are confined to the `agent` repository (the `miru-agent` Rust crate at `agent/agent/`).

## Purpose / Big Picture

`agent/src/app/upgrade.rs` performs an idempotent rebootstrap of on-disk state when the running agent version differs from the marker on disk. It exposes three public entry points:

1. `needs_upgrade(layout, cur_version) -> bool`
2. `reconcile_impl(http_client, layout, version) -> Result<(), UpgradeErr>`
3. `reconcile(layout, http_client, version, sleep_fn) -> ()` (loops forever until success)

The existing `reconcile_returns_uninstalled_err_when_no_device_id_resolvable` test no longer compiles against the current `reconcile_impl` signature (the source no longer calls `storage::resolve_device_id`), so the entire `tests/app/upgrade.rs` file fails to build. The plan stages the work so that after M1 alone the file compiles and the 4 pre-existing `reconcile_*` tests pass; M2–M4 then add 10 new tests (4 `needs_upgrade`, 5 `reconcile_impl`, 1 `reconcile` retry-with-counted-sleep), bringing the post-completion total to 14 passing tests.

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

The Rust crate under test is `miru-agent` rooted at `agent/agent/` (Cargo manifest: `agent/agent/Cargo.toml`, package `miru-agent`).

**Code under test — `agent/agent/src/app/upgrade.rs`:**

- `pub async fn needs_upgrade(layout: &Layout, cur_version: &str) -> bool` (see the `needs_upgrade` function in `agent/src/app/upgrade.rs`). Reads `layout.agent_version()` via `storage::agent_version::read`. Returns:
  - `true` when the marker is missing (`Ok(None)`).
  - `false` when the marker matches `cur_version`.
  - `true` when the marker is present but differs.
  - `true` when the read errors (logs and treats as missing).

- `pub async fn reconcile_impl<HTTPClientT: ClientI>(http_client, layout, version) -> Result<(), UpgradeErr>` (see the `reconcile_impl` function in `agent/src/app/upgrade.rs`). Sequence:
  1. `issue_token` — asserts `auth/private_key` and `auth/public_key` exist, then calls `authn::issue_token` (which calls `http::issue_token` on the client). Returns `UpgradeErr::FileSysErr` on missing key file, `UpgradeErr::AuthnErr` or `UpgradeErr::HTTPErr` on JWT/HTTP failure.
  2. `fetch_device` — `http::devices::get(&token)` then `(&api).into()`. Returns `UpgradeErr::HTTPErr` on failure.
  3. `storage::setup::reset(layout, &device, &Settings::default(), version)` — writes `auth/`, `device.json`, `settings.json`, blank token, wipes `resources/`, writes the agent-version marker LAST so partial failures are recoverable. Returns `UpgradeErr::StorageErr` on failure.
  4. `update_device` — `http::devices::update` with the running version in the body. Returns `UpgradeErr::HTTPErr` on failure.

- `pub async fn reconcile<F, Fut, HTTPClientT: ClientI>(layout, http_client, version, sleep_fn)` (see the `reconcile` function in `agent/src/app/upgrade.rs`). Loop:
  - If `!needs_upgrade(...)` → return.
  - Call `reconcile_impl`. On `Ok` → break. On `Err` → log, increment `err_streak`, sleep via `sleep_fn` for `cooldown::calc(&Backoff { base_secs: 1, growth_factor: 2, max_secs: 60 }, err_streak)` seconds, retry.
  - **Retries on every error type, not just network.** Tests must arrange eventual success or they hang.

**Errors — `agent/agent/src/app/errors.rs`:**

    pub enum UpgradeErr {
        StorageErr(#[from] storage::StorageErr),
        HTTPErr(#[from] http::HTTPErr),
        AuthnErr(#[from] authn::AuthnErr),
        FileSysErr(#[from] filesys::FileSysErr),
    }

**Existing test scaffolding — `agent/agent/tests/app/upgrade.rs`:**

- `prepare_layout(name, device_id) -> (Layout, filesys::Dir)` — creates a temp dir, generates a real RSA keypair under `auth/`, writes a `device.json` with `device_id`. Returns the `Layout` and the `filesys::Dir` (must be held for the dir lifetime).
- `make_mock_client(device) -> Arc<MockClient>` — pre-wires `issue_device_token_fn` to return a 5-minute-future RFC3339 token and `get_device_fn` to return the supplied `backend_client::Device`.
- `backend_device(id, name) -> backend_client::Device` — builds a fixture device.
- `read_keys(layout) -> (String, String)` — reads RSA keypair contents.

**MockClient — `agent/agent/tests/mocks/http_client.rs`:**

- Constructor: `MockClient::default()` plus field overrides via struct update.
- Setters that mutate at runtime: `set_get_device(F)`, `set_update_device(F)`, plus deployment/config/release setters.
- Counters: `num_get_device_calls()`, `num_update_device_calls()`, `call_count(Call::IssueDeviceToken)`.
- Failure injection: closures may return `Err(HTTPErr::MockErr(HTTPMockErr { is_network_conn_err: true }))`.
- `issue_device_token_fn` is a plain `Box<dyn Fn>` (no setter); to override it, construct the `MockClient` directly with the desired closure rather than starting from `make_mock_client`.

**Storage helpers — `agent/agent/src/storage/`:**

- `storage::agent_version::read(file) -> Result<Option<String>, StorageErr>` — returns `Ok(None)` when missing.
- `storage::agent_version::write(file, version) -> Result<(), StorageErr>` — for seeding markers in tests.
- `storage::setup::reset(layout, &Device, &Settings, version) -> Result<(), StorageErr>` — full rebootstrap.

**`Layout` and `filesys::Dir` — `agent/agent/src/storage/layout.rs` / `agent/agent/src/filesys/`:**

- `Layout::new(dir)` wraps a `filesys::Dir`.
- `layout.agent_version()` returns the path to the marker file.
- `layout.auth()` returns a directory wrapper with `.private_key()` and `.public_key()` accessors.
- `layout.device()` returns the `device.json` path wrapper.

## Plan of Work

All edits are in `agent/agent/tests/app/upgrade.rs`. No source changes are required (`reconcile_impl` is already `pub`).

### M1 — Delete the broken test (unblocks compilation)

Delete the function `reconcile_returns_uninstalled_err_when_no_device_id_resolvable` (and its `#[tokio::test]` attribute) from `agent/agent/tests/app/upgrade.rs`. The body references the old `reconcile` return type and prevents the whole file from compiling, so this must happen before any other milestone. After deletion, `cargo test -p miru-agent --test mod -- app::upgrade` should compile and run the four pre-existing `reconcile_*` tests cleanly. Drop any imports that become unused, but note `UpgradeErr` will be re-used by the new M3 tests so it can stay.

### M2 — `needs_upgrade` unit tests (4 cases)

Add these tests at the bottom of `agent/agent/tests/app/upgrade.rs`, after the existing tests. Use `prepare_layout` to get a `Layout`. Import `miru_agent::app::upgrade::needs_upgrade`.

1. `needs_upgrade_returns_true_when_marker_missing` — `prepare_layout`, do not write a marker, assert `needs_upgrade(&layout, "v1.0.0").await == true`.
2. `needs_upgrade_returns_false_when_marker_matches` — `prepare_layout`, `storage::agent_version::write(&layout.agent_version(), "v1.2.3").await.unwrap()`, assert `needs_upgrade(&layout, "v1.2.3").await == false`.
3. `needs_upgrade_returns_true_when_marker_differs` — write `"v1.0.0"`, call with `"v2.0.0"`, expect `true`.
4. `needs_upgrade_returns_true_when_read_errors` — force a read error by creating a directory at the marker path:

        tokio::fs::create_dir_all(layout.agent_version().path()).await.unwrap();

   Note `Layout::agent_version()` returns `filesys::File`; use `.path()` to get the underlying `&Path` for `tokio::fs` calls. The mechanism is deterministic: see the `read` function in `agent/src/storage/agent_version.rs`, which checks `file.exists()` (true for a directory) and then calls `file.read_string().await?`, which fails on a directory and returns `Err(StorageErr::FileSysErr(_))`. Assert `needs_upgrade(...) == true`. If the executor finds this does not error, record it in Surprises & Discoveries — the plan does not provide a fallback because the source path is unambiguous.

### M3 — `reconcile_impl` tests (happy path + one representative failure per pipeline step: FileSysErr from key check, HTTPErr from get_device, StorageErr from reset, HTTPErr from update_device)

Import `miru_agent::app::upgrade::reconcile_impl`. Use `prepare_layout` and `make_mock_client` exactly as the existing `reconcile_*` tests do.

1. `reconcile_impl_happy_path_writes_marker_and_updates_backend` — happy path. Assert `Ok(())`, marker on disk equals `version`, `mock.num_update_device_calls() == 1`, `mock.num_get_device_calls() >= 1`, `mock.call_count(Call::IssueDeviceToken) >= 1`.
2. `reconcile_impl_returns_filesys_err_when_private_key_missing` — `prepare_layout` then `tokio::fs::remove_file(&layout.auth().private_key()).await.unwrap()`. Call `reconcile_impl`. Match `Err(UpgradeErr::FileSysErr(_))` (ultimately surfaced from `private_key_file.assert_exists()?`).
3. `reconcile_impl_returns_http_err_when_get_device_fails` — `mock.set_get_device(|| Err(HTTPErr::MockErr(HTTPMockErr { is_network_conn_err: true })))`. Match `Err(UpgradeErr::HTTPErr(_))`.
4. `reconcile_impl_returns_storage_err_when_reset_fails` — force `storage::setup::reset` to fail by pre-creating a **directory** at `layout.device()`. The mechanism is deterministic: see the `reset` function in `agent/src/storage/setup.rs`, which runs `device_file.write_json(...)` first after creating the auth dir; an atomic JSON write cannot replace an existing directory and returns `Err(StorageErr::FileSysErr(_))`. Steps in the test: after `prepare_layout` (which wrote a file at `layout.device()`), remove the file then create a directory at the same path:

        tokio::fs::remove_file(layout.device().path()).await.unwrap();
        tokio::fs::create_dir_all(layout.device().path()).await.unwrap();

   Note `Layout::device()` returns `filesys::File`; use `.path()` to get the underlying `&Path` for `tokio::fs` calls. Match `Err(UpgradeErr::StorageErr(_))`. If the test fails to surface `UpgradeErr::StorageErr` on first run, log the actual error variant and record it in Surprises & Discoveries before adjusting the failure-injection mechanism.
5. `reconcile_impl_returns_http_err_when_update_device_fails` — happy `get_device`, but `mock.set_update_device(|| Err(HTTPErr::MockErr(HTTPMockErr { is_network_conn_err: true })))`. Match `Err(UpgradeErr::HTTPErr(_))`. Importantly: **the marker has already been written by `setup::reset` before `update_device` runs**, so also assert `storage::agent_version::read(&layout.agent_version()).await.unwrap() == Some(version.to_string())` to lock in the documented "marker last in setup::reset, but update_device is after setup::reset and can fail leaving the marker in place" property of `reconcile_impl`.

### M4 — `reconcile` retry-loop tests

Four already exist (`reconcile_is_noop_when_marker_matches`, `reconcile_rebootstraps_when_marker_missing`, `reconcile_rebootstraps_when_marker_version_differs`, `reconcile_retries_until_get_device_succeeds`). Keep them. Add one more:

1. `reconcile_uses_injected_sleep_and_recovers_after_repeated_failures` — `prepare_layout`. Use `make_mock_client`. Configure `set_get_device` to fail `N = 4` consecutive times then succeed. Inject a counting no-op sleep. Note: `tests/app/upgrade.rs` already imports `chrono::Duration`, so the sleep-fn parameter must use a disambiguated name. Add `use std::time::Duration as StdDuration;` near the top of the file, and write the closure parameter as `StdDuration`. `reconcile`'s bound is `Fn(Duration) -> impl Future<Output = ()> + Send` (not `FnMut`); show the closure as `move |_: StdDuration| { counter.fetch_add(1, Ordering::SeqCst); async {} }`. `Arc<AtomicUsize>::fetch_add` takes `&self`, so the bare `Fn` bound is satisfiable.

        let sleep_count = Arc::new(AtomicUsize::new(0));
        let counter = sleep_count.clone();
        let sleep_fn = move |_: StdDuration| {
            counter.fetch_add(1, Ordering::SeqCst);
            async {}
        };
        reconcile(&layout, mock.as_ref(), "v9.9.9", sleep_fn).await;
        assert_eq!(sleep_count.load(Ordering::SeqCst), 4, "expected exactly 4 sleeps for 4 injected failures");
        assert_eq!(mock.num_update_device_calls(), 1);

This proves the backoff branch is exercised under the injected `sleep_fn` and the loop ultimately terminates.

## Concrete Steps

All commands run from `agent/agent/` unless stated otherwise. The `agent` repo is at `repos/agent/` in the workbench but every command is absolute or relative to `agent/agent/` so they work from a fresh clone.

### Step 0 — Confirm test harness layout

`agent/agent/tests/mod.rs` is the single integration-test binary for the crate; every test module under `agent/agent/tests/` (including `app/upgrade.rs`) is compiled into that one binary via `pub mod` declarations. Therefore the canonical invocation for this plan is:

    cargo test -p miru-agent --test mod

Every `cargo test` line below uses exactly this form. No `--features test` is required (see Decision Log).

### Step 1 — M1 (delete broken test, establish baseline)

Edit `agent/agent/tests/app/upgrade.rs`. Remove the function `reconcile_returns_uninstalled_err_when_no_device_id_resolvable` and its `#[tokio::test]` attribute (lines 200–216 in the pre-edit version). The file currently fails to compile because that function references the old `reconcile_impl` signature — deleting it is what unblocks the rest of this plan. Drop any imports that become unused (note `UpgradeErr` will be re-used by M3 tests, so keep it).

From `agent/agent/`:

    cargo test -p miru-agent --test mod -- app::upgrade

Expected: the file compiles and the four pre-existing `reconcile_*` tests run and pass.

Commit (from `agent/`):

    git add agent/tests/app/upgrade.rs
    git commit -m "test(upgrade): remove obsolete reconcile uninstalled-err test"

### Step 2 — M2 (needs_upgrade tests)

Edit `agent/agent/tests/app/upgrade.rs`. Add `use miru_agent::app::upgrade::needs_upgrade;` to the imports. Append the four `needs_upgrade_*` tests described in Plan of Work / M2.

From `agent/agent/`:

    cargo test -p miru-agent --test mod -- app::upgrade::needs_upgrade

Expected: 4 new tests pass.

Commit (from `agent/`):

    git add agent/tests/app/upgrade.rs
    git commit -m "test(upgrade): cover needs_upgrade missing/match/mismatch/read-error"

### Step 3 — M3 (reconcile_impl tests)

Edit the same file. Add `use miru_agent::app::upgrade::reconcile_impl;` to the imports. Append the five `reconcile_impl_*` tests.

From `agent/agent/`:

    cargo test -p miru-agent --test mod -- app::upgrade::reconcile_impl

Expected: 5 new tests pass.

Commit (from `agent/`):

    git add agent/tests/app/upgrade.rs
    git commit -m "test(upgrade): cover reconcile_impl happy path and per-step failures"

### Step 4 — M4 (reconcile retry test)

Edit the same file. Add the new `reconcile_uses_injected_sleep_and_recovers_after_repeated_failures` test. Bring `std::sync::atomic::{AtomicUsize, Ordering}` and `std::time::Duration as StdDuration` into scope at the top of the file (the `StdDuration` alias avoids a name conflict with the already-imported `chrono::Duration`).

From `agent/agent/`:

    cargo test -p miru-agent --test mod -- app::upgrade::reconcile_uses_injected_sleep

Expected: 1 new test passes; total `app::upgrade::*` count grows by 1.

Commit (from `agent/`):

    git add agent/tests/app/upgrade.rs
    git commit -m "test(upgrade): assert reconcile loop sleeps via injected sleep_fn"

### Step 5 — Full preflight

From `agent/`:

    cargo fmt --all -- --check
    cargo clippy -p miru-agent --tests -- -D warnings
    cargo test -p miru-agent --test mod

Expected: all three commands exit `0`. The full `cargo test` run prints a final line of the form `test result: ok. <N> passed; 0 failed; 0 ignored`. Record `<N>` in Outcomes & Retrospective.

## Validation and Acceptance

A plan reviewer or implementer can verify success by running, from `agent/agent/`:

    cargo test -p miru-agent --test mod -- app::upgrade

and observing all 14 of the following test names in the output, all passing (count is post-deletion of the broken test in M1: 4 `needs_upgrade` + 5 `reconcile_impl` + 4 existing `reconcile` + 1 new counted-sleep `reconcile`):

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

The deleted test `reconcile_returns_uninstalled_err_when_no_device_id_resolvable` MUST NOT appear in the output.

**Preflight must report `clean` before changes are published.** Specifically, from `agent/`:

    cargo fmt --all -- --check
    cargo clippy -p miru-agent --tests -- -D warnings
    cargo test -p miru-agent --test mod

must all exit `0` with no warnings emitted by clippy and no failed or ignored tests. Open the PR only after this preflight is clean.

Acceptance behavior:

- A novice running `cargo test -p miru-agent --test mod` in `agent/agent/` after pulling this branch sees 14 upgrade tests run, all pass; before the branch the file did not compile.
- The `agent/src/app/upgrade.rs` file has the same public API and same line count it did before this work — there are zero source-code changes; only test files change.

## Idempotence and Recovery

- Every step that edits `agent/agent/tests/app/upgrade.rs` is safe to re-run: re-applying the edits is a no-op once the file already contains the new tests, and `cargo test` is naturally idempotent.
- If a test introduced in M2–M4 hangs, the cause is almost certainly a `reconcile` test missing an eventual-success path. Recovery: `Ctrl-C` the test runner, locate the offending test by name in `cargo test -p miru-agent --test mod -- --nocapture app::upgrade`, and ensure the mock's failure injection has a terminating "now succeed" branch. Do not add a `tokio::time::timeout` wrapper — that would introduce wall-clock timing into the test, which the no-real-sleep constraint forbids; fix the mock instead.
- If `cargo clippy --tests -- -D warnings` flags an unused import after the M1 deletion, drop the import and re-run; this is the only common follow-up failure.
- Each milestone ends in its own commit (Steps 1, 2, 3, 4). If preflight fails after a milestone, fix forward in a new commit — do not amend, per the workbench commit policy in `repos/agent/CLAUDE.md`.
