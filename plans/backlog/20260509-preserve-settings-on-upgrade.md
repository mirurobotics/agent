# Preserve user-customized settings across agent package upgrades

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (this repo) | read-write | Modify `agent/src/app/upgrade.rs::reconcile_impl` to read the existing `settings.json` before calling `storage::setup::reset`, falling back to defaults on read failure. Add three integration tests in `agent/tests/app/upgrade.rs`. |

This plan lives in `agent/plans/backlog/` because the work is entirely within the `agent` repo, on the existing branch `fix/preserve-settings-on-upgrade`. The orchestrator will move it to `plans/active/` when implementation begins.

## Purpose / Big Picture

A user who provisions an agent against a non-default backend (for example `staging.api.mirurobotics.com`) currently loses that configuration on every package upgrade. The boot-time upgrade reconciliation in `agent/src/app/upgrade.rs::reconcile_impl` calls `storage::setup::reset(layout, &device, &Settings::default(), version)`, and `storage::setup::reset` (`agent/src/storage/setup.rs:11-31`) atomically rewrites `settings.json` with whatever `Settings` it is given. Passing `Settings::default()` therefore wipes user-set fields like `backend.host` and `mqtt_broker.host` and resets them to prod every upgrade.

After this change, the operator-visible behavior is:

1. A device provisioned with `--backend-host staging.api.mirurobotics.com` (or any other allowed host) keeps that host in `settings.json` after every package upgrade. The agent continues to talk to the same backend it was provisioned against.
2. If `settings.json` is missing or contains unparseable JSON when reconcile runs, the upgrade still completes — the agent falls back to `Settings::default()` and emits a warning in the log so the operator can see why.
3. Newly added settings fields in future agent versions populate from per-field defaults (the existing `Settings` deserializer already does this via `deserialize_warn!` in `agent/src/storage/settings.rs:35-89`), so no migration logic is required.

The fix is small and surgical: roughly ten lines of production code in one function, plus three new tests. Provisioning (`agent/src/provisioning/provision.rs`) and reprovisioning (`agent/src/provisioning/reprovision.rs`) are not touched.

## Progress

- [ ] (YYYY-MM-DD HH:MMZ) Read existing `settings.json` in `reconcile_impl` and pass it through to `storage::setup::reset`; fall back to `Settings::default()` with a warning on read error.
- [ ] Add test `preserves_customized_settings` to `agent/tests/app/upgrade.rs::reconcile_impl` mod.
- [ ] Add test `falls_back_to_defaults_when_settings_missing` to the same mod.
- [ ] Add test `falls_back_to_defaults_when_settings_corrupt` to the same mod.
- [ ] Run `scripts/test.sh` from the agent repo root and confirm all four new test cases pass and the existing five `reconcile_impl` tests still pass.
- [ ] Run `scripts/preflight.sh` from the agent repo root and confirm it prints `Preflight clean`.
- [ ] Commit the change in a single commit with a Conventional Commits subject — see Concrete Steps for the exact commit step.

Use timestamps when you complete steps. Split partially completed work into "done" and "remaining" as needed.

## Surprises & Discoveries

(Add entries as you go.)

- Observation: …
  Evidence: …

## Decision Log

- Decision: Mirror the `get_bootstrap_backend_host` pattern from `agent/src/main.rs:240-247` — `if let Ok(settings) = layout.settings().read_json::<storage::Settings>().await { ... } else { warn!(...); Settings::default() }`.
  Rationale: That helper already establishes the project's idiom for "read settings.json, otherwise default to baseline." Reusing the same shape keeps the codebase consistent and the diff small.
  Date/Author: 2026-05-09, planning subagent.

- Decision: Do not introduce a new helper function. The read-or-default block goes inline at the top of `reconcile_impl`.
  Rationale: It is a single match-or-`if let` block used in exactly one place. Extracting it would add a function and a test point without reducing complexity at the call site. If a second caller appears later, extract then.
  Date/Author: 2026-05-09, planning subagent.

- Decision: On read error the upgrade proceeds with `Settings::default()` instead of failing.
  Rationale: A device that cannot read its `settings.json` is in a bad state, but failing the upgrade leaves it in a worse one (the running binary has changed but the on-disk state has not been reconciled, and `needs_upgrade` will keep returning `true` on every boot). Defaulting and warning matches what `get_bootstrap_backend_host` already does and what the deserializer does when individual fields fail to parse.
  Date/Author: 2026-05-09, planning subagent.

- Decision: Use `tracing::warn!` for the fallback log, matching the existing `warn` import already in `agent/src/app/upgrade.rs:15`.
  Rationale: The file already imports `warn` from `tracing` and uses it for retry messages. No new imports needed.
  Date/Author: 2026-05-09, planning subagent.

- Decision: Tests follow the existing `agent/tests/app/upgrade.rs` harness (`prepare_layout`, `make_mock_client`, `backend_device`) verbatim.
  Rationale: The test file already has the exact harness this work needs. Reuse keeps the new tests stylistically identical to the surrounding ones, and the import linter / field-by-field-assert linter rules already cover this file.
  Date/Author: 2026-05-09, planning subagent.

- Decision: The "preserves customized settings" test uses `staging.api.mirurobotics.com` as the non-default backend host.
  Rationale: That host is allowed by `is_allowed_host` in `agent/src/network/mod.rs:13-17` (it ends with `.mirurobotics.com`), so `BackendHost::new` accepts it and the deserializer round-trips it. It is also the production-realistic value from the bug report.
  Date/Author: 2026-05-09, planning subagent.

- Decision: Validation gate is `scripts/preflight.sh` and it must print `Preflight clean`.
  Rationale: Per `agent/AGENTS.md`, preflight runs the full lint pass (custom import linter, field-by-field-assert detection, `cargo fmt --check`, `cargo machete`, `cargo audit`, `cargo clippy -- -D warnings`), all tests under `--features test`, and covgate enforcement. That is the right gate before publishing the change.
  Date/Author: 2026-05-09, planning subagent.

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

This work happens on the `agent` repository (clone path inside the workbench: `/home/ben/miru/workbench2/repos/agent`). All Rust source paths below are repo-relative to that directory unless prefixed with the workbench root.

### The bug, in one paragraph

`reconcile_impl` is the inner step of the boot-time upgrade reconciliation. It runs whenever the on-disk `agent_version` marker does not match the running binary's version. Today it issues a fresh JWT, fetches the device record from the backend, and then calls `storage::setup::reset(layout, &device, &Settings::default(), version)`. `storage::setup::reset` is shared with `storage::setup::bootstrap` (the first-time provisioning path) and atomically overwrites `settings.json` with whatever `&Settings` it is handed. Because `reconcile_impl` always passes `Settings::default()`, every package upgrade silently resets the operator's customized settings (including `backend.host` and `mqtt_broker.host`) back to prod defaults.

### Files involved

- `agent/src/app/upgrade.rs` — contains `reconcile_impl` (lines 107-117 currently). The single changed function for this work.
- `agent/src/storage/setup.rs` — contains `pub async fn reset(layout, device, settings, agent_version)`. Not modified, but reading lines 11-31 makes clear that the `&Settings` argument is what gets written to `settings.json`.
- `agent/src/storage/settings.rs` — defines `Settings`, `Backend`, `MQTTBroker`. The hand-rolled `Deserialize` impls fill missing fields with per-field defaults (the `deserialize_warn!` macro), so an old `settings.json` missing a newly added field does not fail the read — it warns and uses the new field's default. Not modified.
- `agent/src/main.rs:240-247` — the `get_bootstrap_backend_host` helper this fix mirrors. Pattern: `if let Ok(settings) = settings_file.read_json::<storage::Settings>().await { return settings.backend.host; } storage::Backend::default().host`. Not modified, but referenced by the diff.
- `agent/src/storage/layout.rs:29-31` — `Layout::settings()` returns the `filesys::File` at `<root>/var/lib/miru/settings.json`. The `read_json::<T>()` method is defined on `filesys::File` in `agent/src/filesys/file.rs:129-139`; it reads bytes then `serde_json::from_slice` and returns `Result<T, FileSysErr>` (async).
- `agent/tests/app/upgrade.rs` — existing integration-test file with a working test harness (`prepare_layout`, `make_mock_client`, `backend_device`, `no_sleep`) and an existing `mod reconcile_impl { ... }` block. The three new tests go inside that mod and reuse those helpers.
- `agent/scripts/test.sh` and `agent/scripts/preflight.sh` — the only sanctioned ways to run tests / lint locally. `test.sh` runs `RUST_LOG=off cargo test --features test`. `preflight.sh` runs lint and tests in parallel and prints `Preflight clean` on success.

### Definitions used in this plan

- **Layout** — a struct (`agent/src/storage/layout.rs`) that owns a filesystem root (`filesys::Dir`) and exposes typed accessors for every well-known on-disk path under `<root>/var/lib/miru/`. In tests, `Layout::new(filesys::Dir::create_temp_dir(...))` builds a Layout rooted in a fresh tempdir.
- **`storage::Settings`** — the in-memory shape of `settings.json`. Re-exported at `crate::storage::Settings`. The struct is already imported in `agent/src/app/upgrade.rs:12`.
- **`storage::setup::reset`** — the atomic-rewrite path that wipes per-version state. Signature: `pub async fn reset(layout: &Layout, device: &models::Device, settings: &Settings, agent_version: &str) -> Result<(), StorageErr>`. Not modified.
- **`is_allowed_host`** — the validator behind `BackendHost::new`, defined in `agent/src/network/mod.rs:13-17`. Accepts `mirurobotics.com` exactly or any subdomain ending in `.mirurobotics.com`. `staging.api.mirurobotics.com` is therefore valid.

## Plan of Work

There is exactly one production-code edit and one test-file edit.

### Edit 1: `agent/src/app/upgrade.rs::reconcile_impl`

Current body (lines 107-117):

    pub async fn reconcile_impl<HTTPClientT: ClientI>(
        http_client: &HTTPClientT,
        layout: &Layout,
        version: &str,
    ) -> Result<(), UpgradeErr> {
        let token = issue_token(http_client, layout).await?;
        let device = fetch_device(http_client, &token).await?;
        storage::setup::reset(layout, &device, &Settings::default(), version).await?;
        update_device(http_client, &device, version, &token).await?;
        Ok(())
    }

Replace the `storage::setup::reset` line with a read-or-default block that loads the existing `settings.json` and passes it through. The new body becomes:

    pub async fn reconcile_impl<HTTPClientT: ClientI>(
        http_client: &HTTPClientT,
        layout: &Layout,
        version: &str,
    ) -> Result<(), UpgradeErr> {
        let token = issue_token(http_client, layout).await?;
        let device = fetch_device(http_client, &token).await?;
        let settings = match layout.settings().read_json::<Settings>().await {
            Ok(settings) => settings,
            Err(e) => {
                warn!("unable to read settings.json; falling back to defaults: {e}");
                Settings::default()
            }
        };
        storage::setup::reset(layout, &device, &settings, version).await?;
        update_device(http_client, &device, version, &token).await?;
        Ok(())
    }

No new imports are required: `Settings` is already imported at line 12 (`use crate::storage::{self, Layout, Settings};`), `warn` is already imported at line 15 (`use tracing::{error, info, warn};`), and `read_json` is a method on `filesys::File` which `Layout::settings()` already returns.

The deserializer in `agent/src/storage/settings.rs:35-89` returns `Ok(Settings { ... })` even when individual fields are missing (it warns per missing field and uses `Settings::default()` for that field). It returns `Err` only when the JSON is structurally invalid or fields fail to parse — those error cases land in the `Err` arm above and trigger the warn-and-default fallback.

### Edit 2: `agent/tests/app/upgrade.rs::reconcile_impl` mod

Add three new `#[tokio::test]` functions inside `mod reconcile_impl { ... }` (the existing block ends at line 383). The tests reuse `prepare_layout`, `make_mock_client`, `backend_device`, and the existing `Settings`-related types. Two new imports go at the top of the file alongside the existing `miru_agent::storage::{self, Layout}` import:

    use miru_agent::filesys::WriteOptions;
    use miru_agent::network::{BackendHost, MqttHost};
    use miru_agent::storage::{Backend, MQTTBroker, Settings};

(`PathExt` is already imported via `miru_agent::filesys::{self, Overwrite, PathExt}` on line 11.)

The three tests:

1. `preserves_customized_settings` — pre-populate `layout.settings()` with a `Settings` whose `backend.host` is `BackendHost::new("staging.api.mirurobotics.com").unwrap()` and whose `mqtt_broker.host` is `MqttHost::new("staging.mqtt.mirurobotics.com").unwrap()`. All other fields can be `..Settings::default()`. Run `reconcile_impl(...)` to completion, then read back `layout.settings().read_json::<Settings>().await.unwrap()` and assert (a) `backend.host.as_str() == "staging.api.mirurobotics.com"` and (b) `mqtt_broker.host.as_str() == "staging.mqtt.mirurobotics.com"`. This is the regression-pinning test for the bug.

2. `falls_back_to_defaults_when_settings_missing` — do not write `settings.json`. Run `reconcile_impl(...)` and assert it returns `Ok(())`. Read back `layout.settings().read_json::<Settings>().await.unwrap()` and assert it equals `Settings::default()`. This proves the read-error path completes the upgrade and writes defaults.

3. `falls_back_to_defaults_when_settings_corrupt` — write the literal string `"not-json"` to `layout.settings()` via `write_string("not-json", WriteOptions::OVERWRITE_ATOMIC)` (the same pattern used in `agent/tests/storage/setup.rs:56`). Run `reconcile_impl(...)` and assert `Ok(())`. Read back `settings.json` as `Settings` and assert it equals `Settings::default()`.

A representative skeleton for test 1 (the other two are simpler variations):

    #[tokio::test]
    async fn preserves_customized_settings() {
        let (layout, _dir) = prepare_layout("reconcile_impl_preserves_settings").await;

        let staging = Settings {
            backend: Backend {
                host: BackendHost::new("staging.api.mirurobotics.com").unwrap(),
            },
            mqtt_broker: MQTTBroker {
                host: MqttHost::new("staging.mqtt.mirurobotics.com").unwrap(),
            },
            ..Settings::default()
        };
        layout
            .settings()
            .write_json(&staging, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let mock = make_mock_client(backend_device("dvc_ps1", "preserves"));
        reconcile_impl(mock.as_ref(), &layout, "v1.0.0").await.unwrap();

        let on_disk = layout.settings().read_json::<Settings>().await.unwrap();
        assert_eq!(on_disk.backend.host.as_str(), "staging.api.mirurobotics.com");
        assert_eq!(on_disk.mqtt_broker.host.as_str(), "staging.mqtt.mirurobotics.com");
    }

If clippy or the field-by-field-assert linter flags the two `assert_eq!` calls in any test, suppress with `// lint:allow(field-by-field-assert)` per `agent/AGENTS.md` line 68. Two asserts is below the 4+ threshold, so this should not trigger, but the implementor should be ready for it.

## Concrete Steps

All commands run from `/home/ben/miru/workbench2/repos/agent` (the agent repo root) on branch `fix/preserve-settings-on-upgrade` (already created).

1. Confirm the branch is current and the working tree is clean.

       git status

   Expected: `On branch fix/preserve-settings-on-upgrade` with `nothing to commit, working tree clean`.

2. Apply Edit 1 to `agent/src/app/upgrade.rs::reconcile_impl` exactly as written in Plan of Work above.

3. Apply Edit 2 to `agent/tests/app/upgrade.rs`: add the three imports at the top of the file (only those that are not already present) and the three new `#[tokio::test]` functions inside the existing `mod reconcile_impl { ... }` block.

4. Run the test suite to confirm the three new tests pass and the existing five `reconcile_impl` tests still pass.

       ./scripts/test.sh

   Expected: a final `test result: ok.` line. The five existing `reconcile_impl::*` tests and the three new ones all appear in the running list. If `RUST_LOG=off` swallows the warn message from the corrupt-settings path, that is intentional — the test asserts behavior, not log output.

5. Run preflight to validate lint, format, clippy, machete, audit, all tests, and covgate.

       ./scripts/preflight.sh

   Expected final line: `Preflight clean`. If the line reads `Preflight FAILED (...)`, fix the failing component before committing.

6. Commit the change in a single commit (one milestone, one commit per the policy).

       git add agent/src/app/upgrade.rs agent/tests/app/upgrade.rs
       git commit -m "fix(upgrade): preserve user-customized settings across package upgrades"

   The commit subject is a Conventional Commits `fix` scoped to `upgrade`. Body should explain that `reconcile_impl` now reads `settings.json` before reset and falls back to defaults on error, mirroring `get_bootstrap_backend_host`.

7. Verify a clean status post-commit.

       git status

   Expected: `nothing to commit, working tree clean`. The commit must be the only new commit on the branch.

## Validation and Acceptance

This plan is accepted when all of the following hold:

1. `./scripts/preflight.sh` (run from `/home/ben/miru/workbench2/repos/agent`) prints `Preflight clean` on its final line. **The plan must not be considered complete until preflight reports `clean`.** If preflight fails, the failure must be diagnosed and fixed in this same change set; do not lower covgate thresholds or skip lint rules to make it pass.

2. `./scripts/test.sh` reports the three new tests passing:

       test app::upgrade::reconcile_impl::preserves_customized_settings ... ok
       test app::upgrade::reconcile_impl::falls_back_to_defaults_when_settings_missing ... ok
       test app::upgrade::reconcile_impl::falls_back_to_defaults_when_settings_corrupt ... ok

   …and the existing five `reconcile_impl::*` tests continue to pass.

3. The regression test `preserves_customized_settings` fails on the pre-change codebase (an explicit way to verify: `git stash` the production-code edit, leave the new test in place, run `./scripts/test.sh`, observe the test's `assert_eq!(... "staging.api.mirurobotics.com")` fail because the on-disk host has been reset to the default. Then `git stash pop`.) This is optional verification, not a gate.

4. The diff to production code is confined to `agent/src/app/upgrade.rs` and is approximately ten lines. No changes to `agent/src/provisioning/`, `agent/src/storage/setup.rs`, `agent/src/storage/settings.rs`, or `agent/src/main.rs`.

## Idempotence and Recovery

- Re-running `./scripts/test.sh` and `./scripts/preflight.sh` is safe and idempotent. Both rebuild incrementally and re-run the checks.
- The production-code edit is a localized change to a single function body. If the change is wrong, `git revert <commit-sha>` restores the previous behavior cleanly. There are no migrations, no schema changes, and no on-disk state changes that persist beyond a single test run (every test uses a fresh `tempfile`-backed `Layout`).
- The new tests create their own temp dirs via `prepare_layout(...)` and do not rely on shared state; they can run repeatedly and in parallel with the rest of the suite (no `#[serial]` annotation needed).
- If preflight fails on covgate after the change, do not lower the threshold to make it pass; the new tests should raise or hold coverage in `agent/src/app/`. Investigate any drop instead.
