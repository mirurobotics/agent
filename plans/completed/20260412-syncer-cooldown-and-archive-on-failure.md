# ExecPlan: Divorce syncer cooldown from deployment cooldowns + allow archives on deploy failure

**Status:** active
**Created:** 2026-04-12
**Branch:** `fix/syncer-cooldown-and-archive-on-failure`

## Goal

Two related fixes to the deployment apply/sync pipeline:

1. **Syncer cooldown independence** — The syncer's cooldown should not be inflated by
   individual deployment retry backoff. This allows MQTT-triggered syncs to pull new
   deployments promptly even when an existing deployment is in retry with growing backoff.

2. **Archives on deploy failure** — When the target deployment fails to deploy, old
   deployments needing archival (pure FSM state transitions, no filesystem changes)
   should still be processed. Only removals (filesystem deletions) are blocked.

## Milestones

### M1 — Syncer cooldown independence

**Files:**
- `agent/src/sync/syncer.rs` (lines 173-187)
- `agent/tests/sync/syncer.rs` (tests `deployment_wait_event`, `success_cooldown_over_deployment_wait`)

**Changes:**
1. In `sync()`, when `sync_impl()` returns `Ok(Some(deployment_wait))`:
   - Always use `handle_sync_success()` return value (1s) as the syncer's own cooldown
   - Schedule a **separate** `CooldownEnd::DeploymentWait` notification at `deployment_wait` time
   - The syncer's `cooldown_ends_at` uses only the success wait
   - Return `CooldownEnd::SyncSuccess` as the syncer's own cooldown event

2. Update `deployment_wait_event` test:
   - The syncer now always emits `CooldownEnd::SyncSuccess` for its own cooldown
   - The `CooldownEnd::DeploymentWait` is emitted separately via a spawned notification
   - Verify both events fire: SyncSuccess cooldown end first, then DeploymentWait later

3. Update `success_cooldown_over_deployment_wait` test:
   - With the change, the syncer cooldown is always the success wait regardless of
     deployment wait. Both scenarios now produce `CooldownEnd::SyncSuccess` for the
     syncer's own cooldown, plus a separate `DeploymentWait` notification.
   - This test previously verified that when `success_wait > deployment_wait`, only
     `SyncSuccess` was emitted. Now both tests are symmetric — the syncer's cooldown
     event is always `SyncSuccess`, and `DeploymentWait` fires independently.

### M2 — Archives on deploy failure

**Files:**
- `agent/src/deploy/apply.rs` (lines 51-56)
- `agent/tests/deploy/apply.rs`

**Changes:**
1. In `apply()`, when the target deployment's `apply_one()` returns an error:
   - Instead of returning early with only the target's outcome, also process
     `categorized.archive` and `categorized.wait` deployments via `apply_all()`
   - Skip `categorized.remove` — those involve filesystem deletions that could
     remove files the retrying deployment needs
   - The `dont_remove` parameter doesn't matter for archives (they don't touch files)

2. Update existing tests in `deploy_errors` module:
   - `config_instance_write_permission_denied` (line 709): currently seeds a
     `dpl-to-remove` with `(Archived, Deployed)` → FSM: Remove. The test verifies
     only 1 outcome (the failed target). With the fix, removals are still blocked,
     so this test should remain unchanged.
   - `error_bumps_attempts_and_sets_cooldown` (line 766): same pattern with
     `dpl-to-remove`. Should remain unchanged.
   - `max_retries_exceeded_enters_failed` (line 809): same pattern. Unchanged.

3. Add new tests in `deploy_errors` module:
   - `archives_processed_on_deploy_failure`: seed a target deployment that will fail
     (permission denied) AND an old deployment with `(Archived, Queued)` → FSM: Archive.
     Verify both outcomes: target fails with Retrying, old is archived.
   - `removals_blocked_but_archives_proceed_on_deploy_failure`: seed target (will fail),
     one archive-bound deployment `(Archived, Queued)`, and one remove-bound deployment
     `(Archived, Deployed)`. Verify: 2 outcomes (target error + archived), removal
     is skipped.

### M3 — Validation

1. Run `./scripts/test.sh` — all tests must pass
2. Run `cargo clippy --package miru-agent --all-features -- -D warnings` — no warnings

## Constraints

- Preflight must report clean before publishing
- Follow repo import ordering conventions
- Use `./scripts/test.sh` (which sets `--features test`)
