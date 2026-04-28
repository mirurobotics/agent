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

After this change, every public function in that module has direct unit-test coverage. A novice running `cargo test -p miru-agent` from `agent/agent/` will see the new tests pass and the old broken `reconcile_returns_uninstalled_err_when_no_device_id_resolvable` test gone. The user-visible outcome is a green test run with the new tests included; CI no longer skips upgrade behavior verification.

## Progress

- [ ] M1: Add four `needs_upgrade` tests (missing marker, match, mismatch, read-error).
- [ ] M2: Add five `reconcile_impl` tests (happy path + one failure per step).
- [ ] M3: Verify and (if missing) add one `reconcile` test that asserts the backoff loop is exercised at least N times before final success, using a counting no-op `sleep_fn`.
- [ ] M4: Delete the broken `reconcile_returns_uninstalled_err_when_no_device_id_resolvable` test and confirm `cargo test` still compiles and passes.
- [ ] Final: preflight clean (formatting, clippy, tests).

Use timestamps when you complete steps.

## Surprises & Discoveries

(Add entries as you go.)

## Decision Log

- Decision: Place all new tests in the existing integration test file `agent/agent/tests/app/upgrade.rs` rather than an inline `#[cfg(test)] mod` inside `agent/src/app/upgrade.rs`.
  Rationale: The five existing upgrade tests already live in `tests/app/upgrade.rs` with a working harness (`prepare_layout`, `make_mock_client`, `backend_device`, `read_keys`). Reusing it is simpler than duplicating helpers inline, and keeps coverage discoverable. The "read-error" case for `needs_upgrade` does **not** require module-private access — it can be triggered by making `layout.agent_version()` return a path that exists but contains invalid content (e.g. write a directory in its place, or write bytes that fail the storage decode), so an integration test is sufficient.
  Date/Author: 2026-04-28 / plan author.

- Decision: Do not introduce any new `pub` items in `agent/src/app/upgrade.rs` for testing. The private helpers `issue_token`, `fetch_device`, `update_device` are exercised indirectly through `reconcile_impl`. `reconcile_impl` is already `pub` in the source as of today (verified at `agent/src/app/upgrade.rs:85`).
  Rationale: Avoid widening public surface for tests; the integration tests in `agent/agent/tests/` consume `miru_agent::app::upgrade::*` as a downstream crate.
  Date/Author: 2026-04-28 / plan author.

- Decision: For `reconcile` retry tests, inject `|_| async {}` (a no-op sleep) instead of `tokio::time::sleep`. The three pre-existing `reconcile_*` tests use the real `tokio::time::sleep` because the first retry only waits 1 second; that is fine and we keep it. New retry tests that need many iterations use the no-op sleep to avoid adding wall-clock latency.
  Rationale: Per task constraints — never use real sleep in tests for `reconcile` unless the existing tests already do.
  Date/Author: 2026-04-28 / plan author.

- Decision: Replace (rather than simply delete) the broken `reconcile_returns_uninstalled_err_when_no_device_id_resolvable` test with a `reconcile_impl` test that injects a storage failure (for example, populate `layout.auth().private_key()` with content but make `layout.auth()` non-writable, or pre-create a directory where `device.json` is supposed to be written so `setup::reset` fails) and asserts `Err(UpgradeErr::StorageErr(_))`.
  Rationale: The old test premise — that `reconcile` returns `UpgradeErr::StorageErr(StorageErr::ResolveDeviceIDErr(_))` when no device id is on disk — is dead because `reconcile_impl` no longer calls `storage::resolve_device_id`; the device is fetched from the backend instead. The closest equivalent surface is a `setup::reset` failure inside `reconcile_impl` (which is fallible) — that returns `UpgradeErr::StorageErr` via the `#[from]` conversion. This satisfies M2's "storage failure path" coverage and replaces what the deleted test used to assert.
  Date/Author: 2026-04-28 / plan author.

- Decision: Verify the `test` cargo feature is needed before configuring `cargo test`.
  Rationale: `agent/agent/Cargo.toml` declares `[features] test = []` and the workspace `.vscode/settings.json` enables `"rust-analyzer.cargo.features": ["test"]`. Confirm whether existing `tests/app/upgrade.rs` compiles under `cargo test` without the flag — if the existing tests pass under plain `cargo test -p miru-agent`, the new tests should too. Document the verified invocation in Concrete Steps.
  Date/Author: 2026-04-28 / plan author.

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

The Rust crate under test is `miru-agent` rooted at `agent/agent/` (Cargo manifest: `agent/agent/Cargo.toml`, package `miru-agent`).

**Code under test — `agent/agent/src/app/upgrade.rs`:**

- `pub async fn needs_upgrade(layout: &Layout, cur_version: &str) -> bool` (lines 60–83). Reads `layout.agent_version()` via `storage::agent_version::read`. Returns:
  - `true` when the marker is missing (`Ok(None)`).
  - `false` when the marker matches `cur_version`.
  - `true` when the marker is present but differs.
  - `true` when the read errors (logs and treats as missing).

- `pub async fn reconcile_impl<HTTPClientT: ClientI>(http_client, layout, version) -> Result<(), UpgradeErr>` (lines 85–95). Sequence:
  1. `issue_token` — asserts `auth/private_key` and `auth/public_key` exist, then calls `authn::issue_token` (which calls `http::issue_token` on the client). Returns `UpgradeErr::FileSysErr` on missing key file, `UpgradeErr::AuthnErr` or `UpgradeErr::HTTPErr` on JWT/HTTP failure.
  2. `fetch_device` — `http::devices::get(&token)` then `(&api).into()`. Returns `UpgradeErr::HTTPErr` on failure.
  3. `storage::setup::reset(layout, &device, &Settings::default(), version)` — writes `auth/`, `device.json`, `settings.json`, blank token, wipes `resources/`, writes the agent-version marker LAST so partial failures are recoverable. Returns `UpgradeErr::StorageErr` on failure.
  4. `update_device` — `http::devices::update` with the running version in the body. Returns `UpgradeErr::HTTPErr` on failure.

- `pub async fn reconcile<F, Fut, HTTPClientT: ClientI>(layout, http_client, version, sleep_fn)` (lines 20–58). Loop:
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
- `issue_device_token_fn` is a plain `Box<dyn Fn>` (no setter) — to swap it after construction, build a fresh `MockClient` with the override directly. To swap dynamically, use the existing per-call mutex pattern: call `make_mock_client` then patch via `set_*` setters where available, or for `issue_device_token` just construct with the right closure.

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

### M1 — `needs_upgrade` unit tests (4 cases)

Add these tests at the bottom of `agent/agent/tests/app/upgrade.rs`, after the existing tests. Use `prepare_layout` to get a `Layout`. Import `miru_agent::app::upgrade::needs_upgrade`.

1. `needs_upgrade_returns_true_when_marker_missing` — `prepare_layout`, do not write a marker, assert `needs_upgrade(&layout, "v1.0.0").await == true`.
2. `needs_upgrade_returns_false_when_marker_matches` — `prepare_layout`, `storage::agent_version::write(&layout.agent_version(), "v1.2.3").await.unwrap()`, assert `needs_upgrade(&layout, "v1.2.3").await == false`.
3. `needs_upgrade_returns_true_when_marker_differs` — write `"v1.0.0"`, call with `"v2.0.0"`, expect `true`.
4. `needs_upgrade_returns_true_when_read_errors` — to force a read error, replace the marker file with something the reader cannot parse: e.g. create a directory at the marker path (`tokio::fs::create_dir_all(&layout.agent_version()).await.unwrap()`), so `storage::agent_version::read` returns `Err(_)`. Assert `needs_upgrade(...) == true`. If `agent_version::read` happens to return `Ok(None)` for a directory rather than `Err(_)`, fall back to writing zero-permission bytes (`tokio::fs::write` then `set_permissions` to `0o000`); the test then runs only on Unix — gate with `#[cfg(unix)]` and document the rationale in a one-line comment.

### M2 — `reconcile_impl` tests (happy path + one failure per step)

Import `miru_agent::app::upgrade::reconcile_impl`. Use `prepare_layout` and `make_mock_client` exactly as the existing `reconcile_*` tests do.

1. `reconcile_impl_happy_path_writes_marker_and_updates_backend` — happy path. Assert `Ok(())`, marker on disk equals `version`, `mock.num_update_device_calls() == 1`, `mock.num_get_device_calls() >= 1`, `mock.call_count(Call::IssueDeviceToken) >= 1`.
2. `reconcile_impl_returns_filesys_err_when_private_key_missing` — `prepare_layout` then `tokio::fs::remove_file(&layout.auth().private_key()).await.unwrap()`. Call `reconcile_impl`. Match `Err(UpgradeErr::FileSysErr(_))` (ultimately surfaced from `private_key_file.assert_exists()?`).
3. `reconcile_impl_returns_http_err_when_get_device_fails` — `mock.set_get_device(|| Err(HTTPErr::MockErr(HTTPMockErr { is_network_conn_err: true })))`. Match `Err(UpgradeErr::HTTPErr(_))`.
4. `reconcile_impl_returns_storage_err_when_reset_fails` (this **replaces** the deleted broken test). Force `storage::setup::reset` to fail. Easiest mechanism: pre-create a **directory** at the exact `device.json` path so the atomic JSON write inside `setup::reset` cannot replace a directory. Specifically: after `prepare_layout` (which wrote a file at `layout.device()`), delete that file and `tokio::fs::create_dir_all(&layout.device()).await.unwrap()` to make the path a directory. If the inner write path is different, alternative: revoke write permission on the layout root with `set_permissions(0o555)` (Unix-gated). Match `Err(UpgradeErr::StorageErr(_))`. Add a one-line comment in the test pointing at this plan: `// Replaces the deleted reconcile_returns_uninstalled_err_when_no_device_id_resolvable test; see plans/backlog/20260428-upgrade-tests.md M2.`
5. `reconcile_impl_returns_http_err_when_update_device_fails` — happy `get_device`, but `mock.set_update_device(|| Err(HTTPErr::MockErr(HTTPMockErr { is_network_conn_err: true })))`. Match `Err(UpgradeErr::HTTPErr(_))`. Importantly: **the marker has already been written by `setup::reset` before `update_device` runs**, so also assert `storage::agent_version::read(&layout.agent_version()).await.unwrap() == Some(version.to_string())` to lock in the documented "marker last in setup::reset, but update_device is after setup::reset and can fail leaving the marker in place" property of `reconcile_impl`.

### M3 — `reconcile` retry-loop tests

Three already exist (`reconcile_is_noop_when_marker_matches`, `reconcile_rebootstraps_when_marker_missing`, `reconcile_rebootstraps_when_marker_version_differs`, `reconcile_retries_until_get_device_succeeds`). Keep them. Add one more:

1. `reconcile_uses_injected_sleep_and_recovers_after_repeated_failures` — `prepare_layout`. Use `make_mock_client`. Configure `set_get_device` to fail `N = 4` consecutive times then succeed. Inject a counting no-op sleep:

        let sleep_count = Arc::new(AtomicUsize::new(0));
        let counter = sleep_count.clone();
        let sleep_fn = move |_: Duration| {
            counter.fetch_add(1, Ordering::SeqCst);
            async {}
        };
        reconcile(&layout, mock.as_ref(), "v9.9.9", sleep_fn).await;
        assert!(sleep_count.load(Ordering::SeqCst) >= 4, "expected at least 4 sleeps, got {}", sleep_count.load(Ordering::SeqCst));
        assert_eq!(mock.num_update_device_calls(), 1);

This proves the backoff branch is exercised under the injected `sleep_fn` and the loop ultimately terminates.

### M4 — Delete the broken test

Delete the function `reconcile_returns_uninstalled_err_when_no_device_id_resolvable` (and its `#[tokio::test]` attribute) at lines 200–216 of `agent/agent/tests/app/upgrade.rs`. Also remove now-unused imports if any (`UpgradeErr` is still used by the new M2 tests; `storage::StorageErr` may already be reachable via `storage::`).

## Concrete Steps

All commands run from `agent/agent/` unless stated otherwise. The `agent` repo is at `repos/agent/` in the workbench but every command is absolute or relative to `agent/agent/` so they work from a fresh clone.

### Step 0 — Verify the `test` feature

From `agent/agent/`:

    cargo test -p miru-agent --test mod 2>&1 | head -40

If this errors with "feature `test` is required" or similar missing-symbol errors in `tests/`, switch to:

    cargo test -p miru-agent --features test --test mod 2>&1 | head -40

Record the working invocation in the Decision Log and use it for every subsequent test command in this plan. Expected output: a green run that lists the existing 5 upgrade tests (one of which is the broken one we will delete).

### Step 1 — Establish baseline

From `agent/agent/`:

    cargo test -p miru-agent --test mod -- app::upgrade

Expected: the broken `reconcile_returns_uninstalled_err_when_no_device_id_resolvable` either fails or panics (this is the documented broken state). Record the failure transcript in Surprises & Discoveries.

### Step 2 — M1 (needs_upgrade tests)

Edit `agent/agent/tests/app/upgrade.rs`. Add `use miru_agent::app::upgrade::needs_upgrade;` to the imports. Append the four `needs_upgrade_*` tests described in Plan of Work / M1.

From `agent/agent/`:

    cargo test -p miru-agent --test mod -- app::upgrade::needs_upgrade

Expected: 4 new tests pass.

Commit (from `agent/`):

    git add agent/tests/app/upgrade.rs
    git commit -m "test(upgrade): cover needs_upgrade missing/match/mismatch/read-error"

### Step 3 — M2 (reconcile_impl tests)

Edit the same file. Add `use miru_agent::app::upgrade::reconcile_impl;` to the imports. Append the five `reconcile_impl_*` tests.

From `agent/agent/`:

    cargo test -p miru-agent --test mod -- app::upgrade::reconcile_impl

Expected: 5 new tests pass.

Commit (from `agent/`):

    git add agent/tests/app/upgrade.rs
    git commit -m "test(upgrade): cover reconcile_impl happy path and per-step failures"

### Step 4 — M3 (reconcile retry test)

Edit the same file. Add the new `reconcile_uses_injected_sleep_and_recovers_after_repeated_failures` test. Bring `std::sync::atomic::{AtomicUsize, Ordering}` and `std::time::Duration` into scope at the top of the file.

From `agent/agent/`:

    cargo test -p miru-agent --test mod -- app::upgrade::reconcile_uses_injected_sleep

Expected: 1 new test passes; total `app::upgrade::*` count grows by 1.

Commit (from `agent/`):

    git add agent/tests/app/upgrade.rs
    git commit -m "test(upgrade): assert reconcile loop sleeps via injected sleep_fn"

### Step 5 — M4 (delete broken test)

Edit the same file. Remove `reconcile_returns_uninstalled_err_when_no_device_id_resolvable` (lines 200–216 in the pre-edit version; line numbers will have shifted after M1–M3). Also drop any imports that become unused (e.g. `UpgradeErr` if no longer referenced — but the new M2 tests reference it, so it should stay).

From `agent/agent/`:

    cargo test -p miru-agent --test mod -- app::upgrade
    cargo build -p miru-agent --tests 2>&1 | tail -20

Expected: full upgrade test suite passes; no compile warnings about unused imports.

Commit (from `agent/`):

    git add agent/tests/app/upgrade.rs
    git commit -m "test(upgrade): remove obsolete reconcile uninstalled-err test"

### Step 6 — Full preflight

From `agent/`:

    cargo fmt --all -- --check
    cargo clippy -p miru-agent --tests -- -D warnings
    cargo test -p miru-agent

Expected: all three commands exit `0`. The full `cargo test` run prints a final line of the form `test result: ok. <N> passed; 0 failed; 0 ignored`. Record `<N>` in Outcomes & Retrospective.

## Validation and Acceptance

A plan reviewer or implementer can verify success by running, from `agent/agent/`:

    cargo test -p miru-agent --test mod -- app::upgrade

and observing all of the following test names in the output, all passing:

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
    cargo test -p miru-agent

must all exit `0` with no warnings emitted by clippy and no failed or ignored tests. Open the PR only after this preflight is clean.

Acceptance behavior:

- A novice running `cargo test -p miru-agent` in `agent/agent/` after pulling this branch sees 13 upgrade tests run, all pass; before the branch the run included one broken test that fails.
- The `agent/src/app/upgrade.rs` file has the same public API and same line count it did before this work — there are zero source-code changes; only test files change.

## Idempotence and Recovery

- Every step that edits `agent/agent/tests/app/upgrade.rs` is safe to re-run: re-applying the edits is a no-op once the file already contains the new tests, and `cargo test` is naturally idempotent.
- If a test introduced in M1–M3 hangs, the cause is almost certainly a `reconcile` test missing an eventual-success path. Recovery: `Ctrl-C` the test runner, locate the offending test by name in `cargo test -p miru-agent --test mod -- --nocapture app::upgrade`, and ensure the mock's failure injection has a terminating "now succeed" branch. Add a `tokio::time::timeout(Duration::from_secs(30), reconcile(...)).await.expect("reconcile should terminate")` wrapper as a defensive measure if the test still hangs after fixing the mock.
- If `cargo clippy --tests -- -D warnings` flags an unused import after the M4 deletion, drop the import and re-run; this is the only common follow-up failure.
- If `Step 0` reveals the `test` feature is required and the new tests reference symbols only available without it (or vice versa), document the discovery in Surprises & Discoveries and standardise the whole plan on the working invocation.
- Each milestone ends in its own commit (Steps 2, 3, 4, 5). If preflight fails after a milestone, fix forward in a new commit — do not amend, per the workbench commit policy in `repos/agent/CLAUDE.md`.
