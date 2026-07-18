# Fix: preserve deployment dirty flag when resetting retry state on init

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (this repo, `github.com/mirurobotics/agent`) | read-write | One-line closure fix in `agent/src/disk/mod.rs` plus a regression test in `agent/tests/disk/init.rs`. |

This plan lives in `plans/backlog/` of the agent repo because all changes are made here. Single milestone; no other repos are touched.

## Purpose / Big Picture

The agent persists deployment records in an on-disk cache. Each cache entry carries an `is_dirty` flag meaning "this deployment's status has changed locally and must be pushed to the backend." The sync loop pushes only dirty entries and marks them clean afterward.

`reset_deployment_retry_state` (runs on every agent restart, inside `Storage::init`) rewrites every deployment whose retry state is non-clean — but writes it with an `is_dirty` closure that unconditionally returns `false`. This clobbers the dirty flag on entries that were pending push. Concretely: a deployment fails, its status is written dirty, the agent restarts before the next sync — the restart wipes the dirty flag, and if the retried deployment lands in the same activity/error status, the backend never learns about it. The fleet dashboard shows stale deployment status forever.

After this change, an agent restart still resets retry state (attempts, cooldown) so deployments retry immediately, but any pending status push survives the restart and reaches the backend on the next sync.

## Progress

- [x] Milestone 1: closure fix in `reset_deployment_retry_state`.
- [x] Milestone 1: regression test in `agent/tests/disk/init.rs` (bug-catch verified: fails against `|_, _| false`, passes with fix).
- [x] Milestone 1: `./scripts/test.sh` passes (all `disk::init` tests green; only pre-existing root-environment failures remain — see Surprises & Discoveries). `./scripts/preflight.sh` deferred to the dedicated preflight pass.
- [x] Milestone 1: commit.

## Surprises & Discoveries

- The sandbox runs the suite as root, so permission-based failure tests (chmod `0o555` to force EACCES) fail regardless of this change: 2 lib tests in `deploy::filesys::tests` and 12-13 integration tests (`deploy::*permission_denied*`, `filesys::dirs::new_home_dir::success`, `sync::deployments::apply_error_isolation::*`, occasionally `s3::get::dest_unwritable::*`). Verified identical failures on the untouched branch tip via `git stash`; this change introduces zero new failures. Because `cargo test` fail-fasts per target, the full suite was validated with `--no-fail-fast`.
- The plan's test snippet omitted `.unwrap()` on `dirs::temp(...)` (it returns a `Result`); the implemented test follows the sibling tests' exact style.
- rustfmt wraps the fixed `deployments.write(...)` call across multiple lines (same shape as the idiom's call site at `agent/src/services/deployment/get.rs:42`).

## Decision Log

- Decision: fix the closure as `|old, _| old.is_some_and(|e| e.is_dirty)` (preserve existing dirt, add none) rather than reusing `disk::deployments::is_dirty`.
  Rationale: retry state (`attempts`, `cooldown_ends_at`) is agent-local and never pushed to the backend, so resetting it must not itself create a push — the write should only preserve pre-existing dirt. The chosen closure is the established idiom for exactly this at `agent/src/services/deployment/get.rs:42` and `agent/src/sync/deployments.rs:229`. (`deployments::is_dirty` evaluates identically here since retry fields are not in its diff list, but it expresses "mark dirty on status change," which is not this call site's intent.)
  Date/Author: 2026-07-09 / plan author.

## Outcomes & Retrospective

Implemented as planned, single commit. The write closure in `reset_deployment_retry_state` (`agent/src/disk/mod.rs`) now preserves pre-existing entry dirtiness (`|old, _| old.is_some_and(|e| e.is_dirty)`) instead of forcing `is_dirty = false`, so status pushes pending at restart survive the retry-state reset and reach the backend on the next sync. `make_entry` in `agent/tests/disk/init.rs` takes dirtiness explicitly (four existing call sites pass `false`), and the new `preserves_dirty_flag_on_reset` test covers both the preserved-dirty and untouched-clean cases. Bug-catch verification passed: with the closure temporarily reverted to `|_, _| false`, the new test fails on the `entry.is_dirty` assertion ("dirty flag should survive the retry state reset"); with the fix it passes. `cargo fmt --check` and `cargo clippy -D warnings` are clean. The only test failures in this environment are pre-existing root-user artifacts (documented above), unrelated to this change. Preflight is handled by the dedicated preflight pass before publishing.

## Context and Orientation

Key terms:

- **Deployment**: `agent/src/models/deployment.rs` — a config rollout to this device. Retry state is the `attempts: u32` and `cooldown_ends_at: DateTime<Utc>` fields; `reset_retry_state()` zeroes both; `has_clean_retry_state()` is true when `attempts == 0` and not in cooldown.
- **FileCache**: `agent/src/cache/` — an actor-backed persistent key-value cache. `Deployments` is `cache::FileCache<models::DeploymentID, models::Deployment>` (alias in `agent/src/disk/deployments.rs`). Each stored `CacheEntry` (`agent/src/cache/entry.rs`) has an `is_dirty: bool` field.
- **`write(key, value, is_dirty_fn, overwrite)`**: `agent/src/cache/single_thread.rs` (`SingleThreadCache::write`, exposed on `FileCache` via `agent/src/cache/concurrent.rs`). The closure receives `(Option<&CacheEntry>, &V)` — the existing entry (if any) and the new value — and its return value becomes the persisted entry's `is_dirty`.
- **Dirty flag lifecycle**: deployment state transitions are written dirty via `disk::deployments::is_dirty` (used at `agent/src/deploy/apply.rs:381-384`; helper defined at `agent/src/disk/deployments.rs:8-18`, which preserves `old.is_dirty ||` field diffs). `push_deployments` in `agent/src/sync/deployments.rs` (line ~372-377) fetches `get_dirty_entries()`, pushes only those to the backend, and marks them clean after a successful push.

The bug, at `agent/src/disk/mod.rs:212-225` (called from `Storage::init` at line 134):

    async fn reset_deployment_retry_state(deployments: &Deployments) -> Result<(), StorErr> {
        let entries = deployments
            .find_entries_where(|e| !e.value.has_clean_retry_state())
            .await?;
        for entry in entries {
            let id = entry.key.clone();
            let mut dpl = entry.value;
            dpl.reset_retry_state();
            deployments
                .write(id, dpl, |_, _| false, Overwrite::Allow)
                .await?;
        }
        Ok(())
    }

`|_, _| false` forces `is_dirty = false` on every rewritten entry, discarding pending pushes.

Existing test coverage: `agent/tests/disk/init.rs` (module `reset_retry_state_on_init`) already exercises this path through the public API — it seeds a `deployments.json` via the `make_entry` / `seed_deployments` helpers, runs `Storage::init`, and asserts retry state is reset. But `make_entry` hardcodes `is_dirty: false` and no test asserts entry dirtiness, so the clobbering is invisible to the current suite. Entry dirtiness is observable through the public API via `storage.deployments.read_entry(key)` (returns the full `CacheEntry`).

## Plan of Work

1. `agent/src/disk/mod.rs`, in `reset_deployment_retry_state` (~line 221): change the write closure from `|_, _| false` to `|old, _| old.is_some_and(|e| e.is_dirty)`. Nothing else in the function changes.

2. `agent/tests/disk/init.rs`:
   - Change the `make_entry` helper to take dirtiness explicitly: `fn make_entry(dpl: Deployment, is_dirty: bool) -> CacheEntry<String, Deployment>` and set `is_dirty` from the parameter. Update the four existing call sites to pass `false` (preserving current behavior).
   - Add a test `preserves_dirty_flag_on_reset` to the `reset_retry_state_on_init` module, following the style of the existing tests there:
     - Seed two entries via `seed_deployments`: one deployment `"dpl-dirty-pending"` with `attempts: 3` plus `set_cooldown(TimeDelta::hours(1))`, wrapped in an entry with `is_dirty: true`; and one deployment `"dpl-clean"` with clean retry state (`attempts: 0`, `cooldown_ends_at: UNIX_EPOCH`), wrapped with `is_dirty: false`.
     - Run `Storage::init(&layout, Capacities::default(), "dev".to_string())`.
     - Read both entries back with `storage.deployments.read_entry(<id>.to_string()).await.unwrap()` and assert: the dirty entry has `entry.value.has_clean_retry_state()` true, `entry.value.attempts == 0`, and `entry.is_dirty == true` (the regression assertion — fails before the fix); the clean entry has `entry.is_dirty == false` and `entry.value.has_clean_retry_state()` true (untouched).
     - Use a unique temp dir (`dirs::temp("reset_preserves_dirty")`); no `#[serial]` needed — the test touches no shared OS resources, matching its siblings.

No production code other than the single closure changes. No new modules, so no `.covgate` changes.

## Concrete Steps

All commands run from the repo root, `/home/user/agent`.

1. Apply the edits described in Plan of Work (source fix first, then test).

2. Run the test suite:

       ./scripts/test.sh

   Expect all tests to pass, including the four tests in `disk::init::reset_retry_state_on_init` (three pre-existing plus the new `preserves_dirty_flag_on_reset`). To verify the test actually catches the bug, temporarily revert the closure to `|_, _| false` and re-run — `preserves_dirty_flag_on_reset` must fail on the `entry.is_dirty == true` assertion — then restore the fix.

3. Run preflight:

       ./scripts/preflight.sh

   Expect the final line `Preflight clean` (it runs lint, coverage gates, and the tools lint/tests in parallel and prints each section).

4. Commit (one commit for this single milestone):

       git add agent/src/disk/mod.rs agent/tests/disk/init.rs
       git commit -m "fix: preserve deployment dirty flag when resetting retry state on init"

## Validation and Acceptance

- From `/home/user/agent`, `./scripts/test.sh` passes. The new test `disk::init::reset_retry_state_on_init::preserves_dirty_flag_on_reset` fails before the source change (with `entry.is_dirty` asserted true but observed false) and passes after.
- Behavior accepted: given an on-disk `deployments.json` containing an entry that is both dirty and has non-clean retry state (attempts > 0 and/or an active cooldown), running `Storage::init` produces an entry whose retry state is clean (`attempts == 0`, no cooldown) and whose `is_dirty` is still `true`; a clean, non-dirty entry remains non-dirty with clean retry state.
- `./scripts/preflight.sh` reports `Preflight clean` before the change is published (pushed / PR'd).

## Idempotence and Recovery

All steps are safe to repeat: the edits are deterministic, `./scripts/test.sh` and `./scripts/preflight.sh` are read-only with respect to source, and tests use fresh per-test temp dirs. If a step fails midway, fix and re-run it. To roll back entirely: `git checkout -- agent/src/disk/mod.rs agent/tests/disk/init.rs` (before commit) or `git revert <commit>` (after).
