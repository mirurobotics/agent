# Stop bubbling deployment apply errors to sync-level cooldown

## Purpose

When a deployment fails to apply (e.g. permission denied writing a file), the error currently bubbles up as a `SyncErr`, causing the syncer to increment `err_streak` and set a global cooldown that blocks ALL syncs. A single failing deployment prevents the agent from fetching new deployments.

The per-deployment cooldown already works correctly — `fsm::error()` sets `cooldown_ends_at` on the individual deployment, and `apply.rs` skips deployments in cooldown. The fix is to stop treating deployment apply errors as sync-level errors.

## Progress

- [ ] M1: Fix `apply_deployments()` to not push per-deployment errors into the sync errors vec.
- [ ] M2: Validate — build, test, clippy, lint.

## The Change

In `agent/src/sync/deployments.rs`, the `apply_deployments()` function (lines 285-336) takes `errors: &mut Vec<SyncErr>` and pushes per-deployment apply errors into it at two places:

1. **Line 301:** When `apply::apply()` itself fails (total failure) — this IS a legitimate sync error, keep it.
2. **Line 309:** When individual deployment outcomes have errors — these are per-deployment errors that should NOT be pushed into the sync errors vec. The errors are already logged at line 308, and per-deployment cooldowns are already set by `fsm::error()`.

The fix: remove `errors.push(SyncErr::from(e))` at line 309.

## Concrete Steps

### M1

1. Edit `agent/src/sync/deployments.rs` line 309: remove `errors.push(SyncErr::from(e));` from the per-outcome error handling. The `error!()` log at line 308 stays.

2. Build: `cargo build`
3. Test: `./scripts/test.sh`
4. Commit.

### M2

1. Run `./scripts/lint.sh`
2. Run `cargo clippy --package miru-agent --all-features -- -D warnings`
3. Fix any issues, commit.

## Validation

Preflight must report `clean` before changes are published.

- `cargo build` compiles
- `./scripts/test.sh` passes
- `./scripts/lint.sh` is clean

## What does NOT change

- Syncer cooldown logic in `syncer.rs` — correctly handles real sync failures
- Per-deployment cooldown logic in `fsm.rs` — already works
- Error types or error trait
- Total `apply::apply()` failure at line 297-304 — this is a legitimate sync error
- Pull/push errors — these are legitimate sync errors
