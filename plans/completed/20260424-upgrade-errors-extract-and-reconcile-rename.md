# Refactor `app::upgrade`: extract `UpgradeErr` to `app::errors`, trim docs, rename `ensure` → `reconcile`

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (this repo) | read-write | Move `UpgradeErr` into a new `agent/src/app/errors.rs`, trim verbose comments inside `agent/src/app/upgrade.rs`, and rename `ensure` to `reconcile` in `app::upgrade` plus all of its call sites and test names. |

This plan lives in `agent/plans/backlog/` because the work is entirely within the `agent` repo on the existing branch `feat/idempotent-upgrade-reset`.

## Purpose / Big Picture

This is a pure refactor of the boot-time upgrade gate that landed in PR `feat/idempotent-upgrade-reset`. There is no behavior change — the same function reconciles the on-disk schema with the running binary's version exactly as before. After this change:

1. `UpgradeErr` lives in `agent/src/app/errors.rs`, matching the convention used by every other src module (14 of them: `authn/`, `cache/`, `crypt/`, `deploy/`, `events/`, `filesys/`, `http/`, `models/`, `mqtt/`, `provision/`, `server/`, `services/`, `storage/`, `sync/`). `app/` was the only module without an `errors.rs` because it had no error types until this PR.
2. The entry function's 17-line doc comment shrinks to 4 lines that say what the function does and when it blocks. Two inline comments inside the function body shrink as well — one because the fallback chain is already documented on `resolve_device_id`, the other because the crash-recovery invariant can be stated more tersely.
3. The function name `ensure` becomes `reconcile`, which more accurately describes "no-op-or-fix" semantics and works uniformly for upgrade, downgrade, and missing-marker cases. `reconcile` has strong infra/k8s precedent (controller-style reconciliation loops).

A reader scanning the code after this change should see: a tighter `app::upgrade` module that defers its error type to `app::errors` like every sibling module, a function whose name reads correctly, and only the comments that carry non-obvious information (the PATCH-ordering crash invariant survives — terser).

User-visible behavior: identical. The agent boots, finds an outdated or missing version marker, rebootstraps, PATCHes the backend, and writes the marker — same as today.

## Progress

- [ ] (YYYY-MM-DD HH:MMZ) Part 1 — Create `agent/src/app/errors.rs` containing the `UpgradeErr` enum.
- [ ] Part 1 — Wire `pub mod errors;` into `agent/src/app/mod.rs` and re-export `UpgradeErr` at the module root following sibling-module convention.
- [ ] Part 1 — Remove the `UpgradeErr` enum declaration from `agent/src/app/upgrade.rs` and import the moved type.
- [ ] Part 2 — Replace the 17-line entry-function doc comment with the 4-line version specified in Plan of Work.
- [ ] Part 2 — Trim the `// resolve the device id ...` 4-line comment to the single-line form.
- [ ] Part 2 — Trim the 5-line PATCH-ordering comment to the 3-line terser form.
- [ ] Part 3 — Rename `ensure` to `reconcile` in `agent/src/app/upgrade.rs`.
- [ ] Part 3 — Update the call site in `agent/src/main.rs::run_agent`.
- [ ] Part 3 — Update the import line and 5 callers in `agent/tests/app/upgrade.rs`, plus rename the 5 `ensure_*` test functions to `reconcile_*`.
- [ ] Part 3 — Update the doc-comment reference in `agent/src/authn/issue.rs` line 29 (`app::upgrade::ensure` → `app::upgrade::reconcile`) so links and grep stay accurate.
- [ ] Validation — Run `scripts/preflight.sh` from the repo root and confirm it prints `Preflight clean`.

Use timestamps when you complete steps. Split partially completed work into "done" and "remaining" as needed.

## Surprises & Discoveries

(Add entries as you go.)

- Observation: …
  Evidence: …

## Decision Log

- Decision: New error file is `agent/src/app/errors.rs` (mirroring sibling modules), not `agent/src/app/upgrade_errors.rs` or a free-standing `app::upgrade::errors` submodule.
  Rationale: Every other src module follows the `<module>/errors.rs` pattern. Only one error type exists in `app/` today, but if more app-level error types appear (`StateErr`, `RunErr`, etc.) they all land in the same file alongside `UpgradeErr`. Keeping the location uniform avoids special-casing `app/` for as long as it has any error types.
  Date/Author: 2026-04-24, planning subagent.

- Decision: `is_retryable` stays in `agent/src/app/upgrade.rs` as a private fn rather than moving to `errors.rs`.
  Rationale: It's a retryability classifier specific to the boot-time upgrade-gate retry loop, not a general property of `UpgradeErr`. Other code paths that produce `UpgradeErr` (none today, but in principle) should not inherit a "retry on transport errors only" interpretation. Keeping it in `upgrade.rs` next to `retry_forever` keeps that coupling explicit.
  Date/Author: 2026-04-24, planning subagent.

- Decision: Function rename is `ensure` → `reconcile`, module name stays `app::upgrade`.
  Rationale: `reconcile` reads more accurately than `ensure` for a function that no-ops when state matches and rebootstraps when it doesn't. It also handles upgrade, downgrade, and missing-marker uniformly. The module name `upgrade` stays because the dominant operational case is forward upgrades; renaming the module would ripple unnecessarily.
  Date/Author: 2026-04-24, planning subagent.

- Decision: Test function names track the API rename (`ensure_*` → `reconcile_*`).
  Rationale: Test names exist to read like a spec for the function under test. Leaving `ensure_is_noop_when_marker_matches` after renaming the function would create a permanent stale-name footgun for grep and code review.
  Date/Author: 2026-04-24, planning subagent.

- Decision: Validation gate is `scripts/preflight.sh` and it must print `Preflight clean`.
  Rationale: This is a pure refactor (move + rename + comment trim) with zero functional change. Preflight covers lint, format, all tests, and all covgate modules at threshold — that's the right gate. If covgate thresholds shift more than a fraction of a percent without a structural reason, the implementor should investigate, not adjust the threshold; comment trims and identifier renames do not change executable line coverage.
  Date/Author: 2026-04-24, planning subagent.

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

This work happens on the `agent` repository (clone path inside the workbench: `/home/ben/miru/workbench2/repos/agent`). All Rust source paths below are repo-relative.

### Files involved

- `agent/src/app/upgrade.rs` — current home of `UpgradeErr`, `ensure`, `retry_forever`, and `is_retryable`. The function being renamed and trimmed.
- `agent/src/app/mod.rs` — currently a 4-line file declaring `options`, `run`, `state`, `upgrade` submodules. Has no re-exports today.
- `agent/src/app/state.rs` and `agent/src/app/run.rs` — neighbor files in `app/`. They do not pull anything from `app::upgrade` today; they're cited only as a sanity check that no internal cross-module imports of `UpgradeErr` exist inside `app/`.
- `agent/src/main.rs` — calls `miru_agent::app::upgrade::ensure(&layout, &bootstrap_http_client, version::VERSION).await` from `run_agent` (line 107 at the time of planning). The single non-test call site of the function being renamed.
- `agent/src/authn/issue.rs` — line 29 has a doc comment that references `app::upgrade::ensure` by name. Update for accuracy; not load-bearing for compilation.
- `agent/tests/app/upgrade.rs` — integration test file with the import line `use miru_agent::app::upgrade::{ensure, UpgradeErr};` (line 7) and 5 callers of `ensure(...)`. Five `#[tokio::test]` functions named `ensure_*` need to become `reconcile_*`.
- `scripts/preflight.sh` — the validation gate run from the agent repo root.

### Sibling-module conventions worth matching

Two small examples from sibling modules show how to wire `errors.rs` correctly:

`agent/src/authn/mod.rs` declares the error submodule and re-exports the top-level type:

    pub mod errors;
    pub mod issue;
    pub mod token;
    pub mod token_mngr;

    pub use self::errors::AuthnErr;
    pub use self::issue::issue_token;
    pub use self::token::Token;
    pub use self::token_mngr::{TokenManager, TokenManagerExt};

`agent/src/storage/mod.rs` re-exports two types from its `errors` submodule:

    pub use self::errors::{DeviceNotActivatedErr, StorageErr};

These are the patterns to mirror in `agent/src/app/mod.rs`.

### Retryability stays where it is

`is_retryable` in `agent/src/app/upgrade.rs` is a private function:

    fn is_retryable(err: &UpgradeErr) -> bool {
        matches!(err, UpgradeErr::HTTPErr(_) | UpgradeErr::AuthnErr(_))
    }

It is consumed only by `retry_forever` in the same file. It does not move.

### Glossary

- **Marker file**: the on-disk file at `Layout::agent_version()` that records the version of the agent that last wrote the persistent state.
- **Rebootstrap**: re-fetch the `Device` record from the backend, rewrite `device.json`, `settings.json`, and `auth/token.json`, wipe `resources/` and `events/`, then PATCH the backend with the running version. Implemented by `storage::setup::reset` plus a follow-up PATCH in the upgrade gate.
- **Reconcile**: read on-disk state, compare to the running version, take whatever action (no-op or rebootstrap) makes the two match. The new function name reflects this controller-style "observe → diff → act" pattern.

## Plan of Work

The work is structured as three independent, sequential parts. Each part should compile cleanly on its own; commit after each if convenient.

### Part 1 — Move `UpgradeErr` to `agent/src/app/errors.rs`

1. Create `agent/src/app/errors.rs` with this content (a verbatim move of the enum, plus a header consistent with sibling modules — keep the imports minimal so the file compiles standalone):

        // internal crates
        use crate::authn;
        use crate::filesys;
        use crate::http;
        use crate::storage;

        #[derive(Debug, thiserror::Error)]
        pub enum UpgradeErr {
            #[error(transparent)]
            StorageErr(#[from] storage::StorageErr),
            #[error(transparent)]
            HTTPErr(#[from] http::HTTPErr),
            #[error(transparent)]
            AuthnErr(#[from] authn::AuthnErr),
            #[error(transparent)]
            FileSysErr(#[from] filesys::FileSysErr),
        }

   Note: the existing `upgrade.rs` writes the filesys variant as `crate::filesys::FileSysErr`. Either form is fine; using `filesys::FileSysErr` with a `use crate::filesys;` import keeps the file consistent with how other crates inside the file are referenced.

2. Update `agent/src/app/mod.rs` to declare the submodule and re-export the type:

        pub mod errors;
        pub mod options;
        pub mod run;
        pub mod state;
        pub mod upgrade;

        pub use self::errors::UpgradeErr;

   This matches the `authn/mod.rs` pattern (submodule decls grouped, re-exports below). No other re-exports are added — `state.rs` and `run.rs` are not currently re-exported and this PR does not change that.

3. Edit `agent/src/app/upgrade.rs`:

   - Delete the `UpgradeErr` enum declaration (current lines 15-25).
   - Add `use super::errors::UpgradeErr;` to the internal-crates import block at the top of the file. `super::errors::UpgradeErr` is preferred over `crate::app::errors::UpgradeErr` because every other internal cross-reference inside `upgrade.rs` is via `crate::*` for things outside `app/` (e.g. `crate::authn`, `crate::storage`); using `super::` for the same-module sibling makes the locality explicit.

After Part 1, `cargo check` and `cargo build` from `agent/` must succeed. The test file `agent/tests/app/upgrade.rs` still imports `UpgradeErr` via `miru_agent::app::upgrade::{ensure, UpgradeErr}` — this still resolves because `app::upgrade` does not re-export `UpgradeErr`, but `app::UpgradeErr` does. Update the test import in Part 3 along with the `ensure` rename.

### Part 2 — Trim three comments in `agent/src/app/upgrade.rs`

All three trims happen inside `agent/src/app/upgrade.rs`. They are independent of Part 1 and Part 3.

1. Replace the 17-line doc comment above the entry function (currently lines 27-44, immediately above `pub async fn ensure<...>`) with this 4-line version exactly:

        /// Reconcile on-disk state with the running version. No-op if the marker
        /// matches; otherwise wipes per-version state and rebootstraps from the
        /// backend. Blocks indefinitely on network failure to avoid leaving a
        /// half-wiped device.

   Rationale dropped from the old comment:
   - "Called at boot before `assert_activated`" belongs at the call site in `main.rs` where the ordering is observable, not on the function itself.
   - The "auth/ never touched" note belongs on `storage::setup::reset` where the wipe actually happens — leaving it here would be a stale duplicate.

2. Replace the 4-line inline comment beginning `// resolve the device id from the on-disk state. If the device file is missing or corrupt, fall back to the JWT in the on-disk token...` (currently lines 67-70) with one line:

        // device id (with fallback to the on-disk JWT — see resolve_device_id)

   Rationale: the fallback chain is fully documented on `resolve_device_id` itself in `agent/src/storage/device.rs`. A pointer is enough.

3. Replace the 5-line PATCH-ordering comment (currently lines 99-104) with:

        // PATCH after the marker is on disk: a crash here re-enters next boot,
        // sees the matching marker, and skips the rebootstrap — so the PATCH must
        // succeed within this call or the backend never learns the new version.

   Rationale: the crash-recovery ordering invariant is genuinely non-obvious and is worth keeping; the rewrite says it more tersely without losing the "must succeed within this call" point.

### Part 3 — Rename `ensure` → `reconcile`

1. In `agent/src/app/upgrade.rs`, rename the function declaration `pub async fn ensure<HTTPClientT: ClientI>(...)` to `pub async fn reconcile<HTTPClientT: ClientI>(...)`. No other identifier inside the function body needs to change — `retry_forever`, `is_retryable`, and `cooldown::Backoff` references all stay.

2. In `agent/src/main.rs`, change the single call site at line 107 from `miru_agent::app::upgrade::ensure(&layout, ...)` to `miru_agent::app::upgrade::reconcile(&layout, ...)`. The surrounding `if let Err(e) = ...` and the error-log line `error!("upgrade gate failed: {e}");` stay as-is.

3. In `agent/src/authn/issue.rs` line 29, change the doc-comment reference `app::upgrade::ensure` to `app::upgrade::reconcile`. This file is not load-bearing for compilation but the doc accuracy matters for grep and rustdoc links.

4. In `agent/tests/app/upgrade.rs`:
   - Change the import line from `use miru_agent::app::upgrade::{ensure, UpgradeErr};` to `use miru_agent::app::upgrade::reconcile;` and add a separate `use miru_agent::app::UpgradeErr;` line. This second import flows through the new `pub use self::errors::UpgradeErr;` in `app/mod.rs`. Keeping it as a separate `use` makes the move visible in diffs.
   - Update the 5 callers of `ensure(&layout, mock.as_ref(), "...")` to `reconcile(&layout, mock.as_ref(), "...")`.
   - Rename the 5 test functions:
     - `ensure_is_noop_when_marker_matches` → `reconcile_is_noop_when_marker_matches`
     - `ensure_rebootstraps_when_marker_missing` → `reconcile_rebootstraps_when_marker_missing`
     - `ensure_rebootstraps_when_marker_version_differs` → `reconcile_rebootstraps_when_marker_version_differs`
     - `ensure_retries_until_get_device_succeeds` → `reconcile_retries_until_get_device_succeeds`
     - `ensure_returns_uninstalled_err_when_no_device_id_resolvable` → `reconcile_returns_uninstalled_err_when_no_device_id_resolvable`
   - Update the inline comment inside `reconcile_is_noop_when_marker_matches` that says "ensure() should make zero HTTP calls" to "reconcile() should make zero HTTP calls", and the doc comment on `prepare_layout` that mentions "the JWT-signing path inside `ensure`" → "...inside `reconcile`".

After Part 3, `cargo build`, `cargo test`, and `cargo clippy` (whatever preflight runs) must succeed.

## Concrete Steps

All commands run from the agent repo root: `/home/ben/miru/workbench2/repos/agent` (the planning subagent verified `git rev-parse --abbrev-ref HEAD` reports `feat/idempotent-upgrade-reset`).

### Verify the starting state

    git status
    git rev-parse --abbrev-ref HEAD     # expect: feat/idempotent-upgrade-reset
    grep -n "UpgradeErr\|ensure\|reconcile" agent/src/app/upgrade.rs agent/src/main.rs agent/tests/app/upgrade.rs agent/src/authn/issue.rs

Expected: `UpgradeErr` and `ensure` appear in `app/upgrade.rs`, `main.rs`, `tests/app/upgrade.rs`, and `authn/issue.rs` (doc comment only). `reconcile` does not appear anywhere yet.

### Part 1 — extract errors

1. Create `agent/src/app/errors.rs` with the content shown in Plan of Work step 1.
2. Edit `agent/src/app/mod.rs` per Plan of Work step 2.
3. Edit `agent/src/app/upgrade.rs` per Plan of Work step 3.
4. Sanity-check:

        cargo check -p miru-agent

   Expected: a clean compile. If it fails complaining about an unused `use storage::*` or `use http::*`, audit the imports inside `errors.rs` against the `#[from]` types — the four variants need exactly `storage::StorageErr`, `http::HTTPErr`, `authn::AuthnErr`, and `filesys::FileSysErr`.

### Part 2 — trim comments

5. Replace the three comments in `agent/src/app/upgrade.rs` per Plan of Work part 2.
6. Sanity-check:

        cargo check -p miru-agent

### Part 3 — rename `ensure` → `reconcile`

7. Edit `agent/src/app/upgrade.rs`, `agent/src/main.rs`, `agent/src/authn/issue.rs`, and `agent/tests/app/upgrade.rs` per Plan of Work part 3.
8. Confirm the rename is complete:

        grep -rn "ensure\b" agent/src/app agent/src/main.rs agent/tests/app/upgrade.rs agent/src/authn/issue.rs

   Expected: no hits. (The word `ensure` may still appear unrelated elsewhere in the repo — e.g. in other modules or in third-party crate names — that's fine. Scope the grep to the files this plan touches.)

        grep -rn "reconcile\b" agent/src/app agent/src/main.rs agent/tests/app/upgrade.rs

   Expected: declarations and call sites visible.

### Validation — preflight

9. Run the validation gate:

        ./scripts/preflight.sh

   Expected final line: `Preflight clean`.

   If preflight fails:
   - **Compile or clippy errors**: fix them. Most likely culprit is a missed import or a stale `ensure` reference somewhere not covered by the grep above.
   - **Test failures**: the rename in `tests/app/upgrade.rs` may have left a `cargo test` runner unable to find a renamed function in a `#[ignore]` filter or similar. Re-run `cargo test --workspace` and read the failure.
   - **Covgate threshold drift**: this is a pure refactor — comment trims and identifier renames do not change executable line coverage. If a covgate module shifts by more than a fraction of a percent, **investigate the structural reason** before adjusting the threshold. Likely candidates: a moved `match` arm, an inadvertently dropped `#[from]` variant, or a test that no longer runs because of a typo in the renamed function name. Do not bump thresholds to make it pass.
   - **Format**: `cargo fmt --all` from the repo root and re-run preflight.

### Commit suggestion

Three commits map cleanly to the three parts; the implementor may collapse to one if preferred. Suggested messages:

    refactor(app): move UpgradeErr to app::errors, mirror sibling-module convention
    docs(upgrade): trim entry-fn comment and two inline comments
    refactor(app::upgrade): rename ensure → reconcile to match controller-style semantics

Each commit must be made from inside `/home/ben/miru/workbench2/repos/agent` (not the workbench root).

## Validation and Acceptance

Acceptance is observable only as "preflight stays clean and behavior is unchanged."

- Preflight: from `/home/ben/miru/workbench2/repos/agent`, run `./scripts/preflight.sh`. Expected output ends with `Preflight clean`. This single command exercises lint, format, the full `cargo test` suite, and all covgate modules at their thresholds.
- Spot-check the 5 renamed tests pass:

        cargo test --test app -- reconcile_

   Expected: 5 tests run, 5 pass.

- Spot-check no stale `ensure` symbol survives where it should not:

        grep -rn "app::upgrade::ensure\|fn ensure\b\|use.*upgrade::ensure" agent/

   Expected: no hits.

- Behavior unchanged: there is no runtime acceptance test beyond the unit/integration tests, because the change is a refactor. The 5 tests in `agent/tests/app/upgrade.rs` cover the four operational paths (no-op, missing marker, version mismatch, transient network failure) and the one fatal path (uninstalled). All five must pass after the rename.

## Idempotence and Recovery

- All edits are textual and reversible. If preflight fails after Part 3, `git diff` shows exactly what changed in the rename commit; `git checkout -- <file>` reverts a single file safely.
- Creating `agent/src/app/errors.rs` is safe to re-run: the file does not exist before this plan and the `Write` tool will overwrite if the implementor needs to rewrite it.
- The `pub mod errors;` line in `agent/src/app/mod.rs` is idempotent — adding it twice is a compile error that surfaces immediately at `cargo check`.
- The rename is search-and-replace; if the implementor inadvertently renames an unrelated `ensure` (none exist in the touched files at the time of planning, but be careful with editor-wide replace), `grep` plus `git diff` will catch it before the commit.
- If a covgate threshold genuinely needs adjustment (it shouldn't — this is a refactor), record the structural reason in Surprises & Discoveries and the Decision Log before changing the threshold value.
