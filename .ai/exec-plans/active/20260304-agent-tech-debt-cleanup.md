# Agent Tech Debt Cleanup (TD-001 through TD-007)

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Purpose / Big Picture

Resolve all seven documented tech debt items in `agent/TECH_DEBT.md`. After this work, the agent codebase will have consistent import comment labels across all source files, a conventional module layout for `cli`, no dead code in test utilities or suppressed-import annotations, and reduced boilerplate in model enum conversions, the cache actor dispatch, and the shutdown manager. Each item is committed separately so changes are reviewable and revertable independently.

The user can verify success by running `agent/scripts/test.sh` (all tests pass), `agent/scripts/lint.sh` (no warnings), and confirming `agent/TECH_DEBT.md` is empty of items.

**Workflow gate:** Each milestone must be reviewed by the user, accepted, and committed before proceeding to the next milestone. Do not begin work on milestone N+1 until milestone N is committed. Present the changes to the user and wait for their approval at each gate.

## Progress

- [ ] Milestone 1: TD-003 — Remove unused test utilities
- [ ] Gate 1: User review + commit
- [ ] Milestone 2: TD-004 — Remove dead tracing imports
- [ ] Gate 2: User review + commit
- [ ] Milestone 3: TD-002 — Restructure cli module
- [ ] Gate 3: User review + commit
- [ ] Milestone 4: TD-001 — Normalize import comment labels
- [ ] Gate 4: User review + commit
- [ ] Milestone 5: TD-007 — Consolidate ShutdownManager handle methods
- [ ] Gate 5: User review + commit
- [ ] Milestone 6: TD-006 — Reduce cache actor dispatch boilerplate
- [ ] Gate 6: User review + commit
- [ ] Milestone 7: TD-005 — Macro-ify model enum conversions
- [ ] Gate 7: User review + commit
- [ ] Final: Remove resolved items from TECH_DEBT.md, run full test + lint
- [ ] Gate 8: User review + commit

## Surprises & Discoveries

(Add entries as you go.)

## Decision Log

- Decision: Order milestones from smallest/safest to largest/riskiest.
  Rationale: XS dead-code and structure items are mechanical with no behavioral change. The complexity items (TD-005, TD-006, TD-007) introduce new macros or refactors that carry more risk and benefit from a clean baseline.
  Date/Author: 2026-03-04

- Decision: For TD-001, adopt `// standard crates`, `// internal crates`, `// external crates` as the canonical labels and update AGENTS.md to match.
  Rationale: User preference. `// internal crates` (39 files) and `// external crates` (33 files) are already the majority. Switching `// standard library` (22 files) to `// standard crates` (4 files) aligns all three groups to the consistent `// xxx crates` pattern. ~26 files need label updates (the 22 using `// standard library` plus the other outliers).
  Date/Author: 2026-03-04

- Decision: For TD-001, classify `backend_api` and `device_api` (generated workspace-sibling crates) as internal imports.
  Rationale: They are workspace members defined in the agent's `Cargo.toml` workspace, not external registry crates. Most files already place them under `// internal crates`.
  Date/Author: 2026-03-04

- Decision: Each milestone requires user review, acceptance, and commit before proceeding to the next.
  Rationale: User preference — ensures each change is inspected and committed atomically before building on it.
  Date/Author: 2026-03-04

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

### Repository layout

The agent is a Rust workspace at `agent/` (submodule root) containing:

- `agent/agent/src/` — 22 modules listed in `agent/agent/src/lib.rs`, plus `main.rs`
- `agent/agent/tests/` — mirror of source modules with external integration tests
- `agent/AGENTS.md` — coding conventions (import ordering, error handling, module layout)
- `agent/ARCHITECTURE.md` — system design and invariants
- `agent/TECH_DEBT.md` — the 7 items this plan resolves
- `agent/scripts/test.sh` — canonical test runner: `RUST_LOG=off cargo test --features test -- --test-threads=1`
- `agent/scripts/lint.sh` — canonical lint runner (fmt, clippy, machete, audit)

### Import comment convention (current state)

AGENTS.md documents: `// standard library`, `// internal`, `// external`. The codebase actually uses a mix of labels. Counting all 113 import-group comment lines across source files:

| Group | Majority label | Count | Outlier labels | Count |
|-------|---------------|-------|----------------|-------|
| std   | `// standard library` | 22 | `// standard crates` | 4 |
| internal | `// internal crates` | 39 | `// internal` (1), `// internal modules` (2) | 3 |
| external | `// external crates` | 33 | `// external` (4), `// external libraries` (1) | 5 |

Target convention: `// standard crates`, `// internal crates`, `// external crates`.

### Key files for each tech debt item

| Item | Files to change |
|------|----------------|
| TD-001 | AGENTS.md (update labels), ~12 source files with outlier labels |
| TD-002 | `agent/src/cli.rs` → `agent/src/cli/mod.rs`, new `agent/tests/cli/mod.rs`, new `agent/src/cli/.covgate` |
| TD-003 | `agent/tests/test_utils/testdata.rs` (remove 3 functions) |
| TD-004 | `agent/src/installer/display.rs`, `agent/src/telemetry/mod.rs`, `agent/src/storage/layout.rs` (remove 2 lines each) |
| TD-005 | `agent/src/models/deployment.rs`, `agent/src/models/device.rs`, new macro location TBD |
| TD-006 | `agent/src/cache/concurrent.rs` (add helper macro, simplify match arms) |
| TD-007 | `agent/src/app/run.rs` (consolidate 3 worker handle methods) |

## Plan of Work

### Milestone 1: TD-003 — Remove unused test utilities

In `agent/tests/test_utils/testdata.rs`, delete the three functions `filesys_testdata_dir()`, `sandbox_testdata_dir()`, and `crypt_testdata_dir()` (lines 11-21). Keep `testdata_dir()` (lines 5-9) which is actually used. Run tests to confirm nothing breaks.

### Milestone 2: TD-004 — Remove dead tracing imports

In each of these three files, remove both the `#[allow(unused_imports)]` annotation and the `use tracing::{...};` line:

1. `agent/src/installer/display.rs` — remove lines 2-3 (`#[allow(unused_imports)]` and `use tracing::{debug, error, info, warn};`)
2. `agent/src/telemetry/mod.rs` — remove lines 2-3 (`#[allow(unused_imports)]` and `use tracing::{debug, error, info, trace, warn};`)
3. `agent/src/storage/layout.rs` — remove lines 5-6 (`#[allow(unused_imports)]` and `use tracing::{debug, error, info, warn};`)

Run lint (clippy) to confirm no new warnings.

### Milestone 3: TD-002 — Restructure cli module

1. Create directory `agent/src/cli/`.
2. Move `agent/src/cli.rs` to `agent/src/cli/mod.rs`.
3. Remove the inline `#[cfg(test)] mod tests { ... }` block (lines 50-179) from `agent/src/cli/mod.rs`.
4. Create `agent/tests/cli/mod.rs` containing the test code extracted from the source file. Adjust imports: replace `use super::*;` with `use miru_agent::cli::{Args, InstallArgs};`. Keep the `to_inputs` helper and both test submodules (`args_parse`, `install_args_parse`).
5. Create `agent/src/cli/.covgate` with an appropriate threshold (e.g. `90.00` — the module is small and fully tested).
6. Run tests to confirm all 7 cli tests pass.

### Milestone 4: TD-001 — Normalize import comment labels

Update AGENTS.md to document the target convention:

    // standard crates
    use std::sync::Arc;

    // internal crates
    use crate::app::state::AppState;

    // external crates
    use tokio::sync::broadcast;

Then fix all files that use outlier labels. There are two groups of changes:

**Group A — `// standard library` → `// standard crates`** (22 files):

| File | Current label |
|------|---------------|
| `main.rs:1` | `// standard library` |
| `logs/mod.rs:1` | `// standard library` |
| `cache/single_thread.rs:1` | `// standard library` |
| `cache/entry.rs:1` | `// standard library` |
| `cache/dir.rs:1` | `// standard library` |
| `cache/file.rs:1` | `// standard library` |
| `cache/concurrent.rs:1` | `// standard library` |
| `app/state.rs:1` | `// standard library` |
| `app/options.rs:1` | `// standard library` |
| `app/run.rs:1` | `// standard library` |
| `filesys/path.rs:1` | `// standard library` |
| `filesys/cached_file.rs:1` | `// standard library` |
| `filesys/errors.rs:1` | `// standard library` |
| `filesys/dir.rs:1` | `// standard library` |
| `filesys/file.rs:1` | `// standard library` |
| `cooldown/mod.rs:1` | `// standard library` |
| `crypt/rsa.rs:1` | `// standard library` |
| `storage/mod.rs:1` | `// standard library` |
| `installer/install.rs:1` | `// standard library` |
| `authn/token_mngr.rs:1` | `// standard library` |
| `mqtt/client.rs:1` | `// standard library` |
| `activity/mod.rs:1` | `// standard library` |
| `http/client.rs:1` | `// standard library` |
| `http/request.rs:1` | `// standard library` |
| `server/state.rs:1` | `// standard library` |
| `server/serve.rs:1` | `// standard library` |
| `workers/token_refresh.rs:1` | `// standard library` |

**Group B — other outlier labels** (8 occurrences across 6 files):

| File | Current label | Target label |
|------|---------------|--------------|
| `main.rs:4` | `// internal` | `// internal crates` |
| `main.rs:18` | `// external` | `// external crates` |
| `workers/poller.rs:7` | `// internal modules` | `// internal crates` |
| `workers/mqtt.rs:7` | `// internal modules` | `// internal crates` |
| `app/run.rs:25` | `// external` | `// external crates` |
| `server/serve.rs:17` | `// external` | `// external crates` |
| `server/handlers.rs:14` | `// external` | `// external crates` |
| `crypt/rsa.rs:10` | `// external libraries` | `// external crates` |

Run lint to verify no impact.

### Milestone 5: TD-007 — Consolidate ShutdownManager handle methods

In `agent/src/app/run.rs`, the three worker handle methods (`with_token_refresh_worker_handle`, `with_poller_worker_handle`, `with_mqtt_worker_handle`) all accept `JoinHandle<()>` and follow an identical check-and-set pattern. Replace them with a single private helper:

    fn register_worker_handle(
        slot: &mut Option<JoinHandle<()>>,
        name: &str,
        handle: JoinHandle<()>,
    ) -> Result<(), ServerErr> {
        if slot.is_some() {
            return Err(ServerErr::ShutdownMngrDuplicateArgErr(
                ShutdownMngrDuplicateArgErr {
                    arg_name: name.to_string(),
                    trace: trace!(),
                },
            ));
        }
        *slot = Some(handle);
        Ok(())
    }

Then replace each of the three methods with a one-liner delegation:

    pub fn with_token_refresh_worker_handle(&mut self, h: JoinHandle<()>) -> Result<(), ServerErr> {
        Self::register_worker_handle(&mut self.token_refresh_worker_handle, "token_refresh_handle", h)
    }

Keep `with_app_state()` and `with_socket_server_handle()` as-is since they have different signatures. Run tests.

### Milestone 6: TD-006 — Reduce cache actor dispatch boilerplate

In `agent/src/cache/concurrent.rs`, add a helper macro at the top of the file (after imports, before the traits):

    macro_rules! dispatch {
        ($self:expr, $method:ident ( $($arg:expr),* ), $respond_to:expr, $msg:expr) => {{
            let result = $self.cache.$method( $($arg),* ).await;
            if $respond_to.send(result).is_err() {
                error!($msg);
            }
        }};
    }

Then rewrite each of the 20 non-Shutdown arms in `Worker::run()` from:

    WorkerCommand::Read { key, respond_to } => {
        let result = self.cache.read(&key).await;
        if respond_to.send(result).is_err() {
            error!("Actor failed to read cache entry");
        }
    }

To:

    WorkerCommand::Read { key, respond_to } => {
        dispatch!(self, read(&key), respond_to, "Actor failed to read cache entry");
    }

The `Shutdown` arm remains unchanged (it has `break`). Run tests to confirm behavior is preserved.

### Milestone 7: TD-005 — Macro-ify model enum conversions

This is the largest item. Create a new declarative macro `impl_model_enum!` in `agent/src/models/mod.rs` (or a dedicated `agent/src/models/macros.rs` if cleaner). The macro generates:

1. Custom `Deserialize` impl with warn-and-default on unknown variants
2. `variants()` method returning `Vec<Self>`
3. `From<&Self>` for one or two generated-API types
4. `From<&GeneratedType>` for Self (reverse conversion)

Macro invocation shape (example for `DplTarget`):

    impl_model_enum! {
        #[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq, Hash)]
        #[serde(rename_all = "snake_case")]
        pub enum DplTarget {
            #[default]
            Staged => "staged",
            Deployed => "deployed",
            Archived => "archived",
        }
        warn_prefix: "deployment target status",
        convert_to: [
            agent_server::DeploymentTargetStatus {
                Staged => DEPLOYMENT_TARGET_STATUS_STAGED,
                Deployed => DEPLOYMENT_TARGET_STATUS_DEPLOYED,
                Archived => DEPLOYMENT_TARGET_STATUS_ARCHIVED,
            },
            backend_client::DeploymentTargetStatus {
                Staged => DEPLOYMENT_TARGET_STATUS_STAGED,
                Deployed => DEPLOYMENT_TARGET_STATUS_DEPLOYED,
                Archived => DEPLOYMENT_TARGET_STATUS_ARCHIVED,
            },
        ],
        convert_from: [
            backend_client::DeploymentTargetStatus {
                DEPLOYMENT_TARGET_STATUS_STAGED => Staged,
                DEPLOYMENT_TARGET_STATUS_DEPLOYED => Deployed,
                DEPLOYMENT_TARGET_STATUS_ARCHIVED => Archived,
            },
        ],
    }

Apply the macro to all five enums: `DplTarget`, `DplActivity`, `DplErrStatus`, `DplStatus` (in `deployment.rs`), and `DeviceStatus` (in `device.rs`). Keep `DplStatus::from_activity_and_error()` as a manual impl since it has custom logic.

Run tests thoroughly — the model enums are used everywhere.

### Final: Clean up TECH_DEBT.md

Remove all 7 items and their table rows from `agent/TECH_DEBT.md`, leaving only the header. Run full test suite and lint one final time.

## Concrete Steps

All commands run from the `agent/` directory (submodule root) unless otherwise noted.

### Milestone 1 (TD-003)

    # Edit agent/tests/test_utils/testdata.rs — remove lines 10-21 (the blank line + 3 functions)
    # Verify
    From agent/: ./scripts/test.sh

Expected: all tests pass; no test calls these functions.
**GATE:** Present changes to user. Wait for review, acceptance, and commit before proceeding.

### Milestone 2 (TD-004)

    # Edit 3 files — remove the #[allow(unused_imports)] + use tracing lines
    # Verify
    From agent/: cargo clippy --package miru-agent --all-features -- -D warnings

Expected: zero warnings.
**GATE:** Present changes to user. Wait for review, acceptance, and commit before proceeding.

### Milestone 3 (TD-002)

    From agent/: mkdir -p agent/src/cli
    From agent/: mv agent/src/cli.rs agent/src/cli/mod.rs
    # Edit agent/src/cli/mod.rs — remove #[cfg(test)] mod tests block
    # Create agent/tests/cli/mod.rs with extracted tests
    # Create agent/src/cli/.covgate with content "90.00"
    # Verify
    From agent/: ./scripts/test.sh

Expected: all tests pass including the 7 cli tests.
**GATE:** Present changes to user. Wait for review, acceptance, and commit before proceeding.

### Milestone 4 (TD-001)

    # Edit AGENTS.md — update import ordering example to use "// standard crates", "// internal crates", "// external crates"
    # Edit ~27 source files with "// standard library" → "// standard crates" (Group A)
    # Edit ~6 source files with other outlier labels (Group B)
    # Verify
    From agent/: cargo fmt -p miru-agent -- --check
    From agent/: cargo clippy --package miru-agent --all-features -- -D warnings

Expected: no formatting or lint issues.
**GATE:** Present changes to user. Wait for review, acceptance, and commit before proceeding.

### Milestone 5 (TD-007)

    # Edit agent/src/app/run.rs — add register_worker_handle helper, simplify 3 methods
    # Verify
    From agent/: ./scripts/test.sh

Expected: all tests pass.
**GATE:** Present changes to user. Wait for review, acceptance, and commit before proceeding.

### Milestone 6 (TD-006)

    # Edit agent/src/cache/concurrent.rs — add dispatch! macro, simplify 20 match arms
    # Verify
    From agent/: ./scripts/test.sh

Expected: all tests pass; cache tests specifically should all pass.
**GATE:** Present changes to user. Wait for review, acceptance, and commit before proceeding.

### Milestone 7 (TD-005)

    # Create or edit agent/src/models/mod.rs — add impl_model_enum! macro
    # Edit agent/src/models/deployment.rs — replace ~350 lines of boilerplate with macro invocations
    # Edit agent/src/models/device.rs — replace DeviceStatus boilerplate with macro invocation
    # Verify
    From agent/: ./scripts/test.sh
    From agent/: cargo clippy --package miru-agent --all-features -- -D warnings

Expected: all tests pass; zero clippy warnings.
**GATE:** Present changes to user. Wait for review, acceptance, and commit before proceeding.

### Final

    # Edit agent/TECH_DEBT.md — remove all 7 items, leaving only the empty header
    # Full verification
    From agent/: ./scripts/test.sh
    From agent/: ./scripts/lint.sh

**GATE:** Present changes to user. Wait for review, acceptance, and commit.

Expected: all tests pass, lint clean.

## Validation and Acceptance

1. `./scripts/test.sh` passes with zero failures after every milestone and at the end.
2. `./scripts/lint.sh` passes clean (fmt, clippy, machete, audit) at the end.
3. `agent/TECH_DEBT.md` contains only the header table with zero rows.
4. `agent/AGENTS.md` import ordering example uses `// standard crates`, `// internal crates`, `// external crates`.
5. No file in `agent/src/` uses the labels `// standard library`, `// internal`, `// external`, `// internal modules`, or `// external libraries`.
6. `agent/src/cli/mod.rs` exists (not `cli.rs`), `agent/tests/cli/mod.rs` exists, `agent/src/cli/.covgate` exists.
7. `agent/tests/test_utils/testdata.rs` contains only `testdata_dir()`.
8. No `#[allow(unused_imports)]` appears in `installer/display.rs`, `telemetry/mod.rs`, or `storage/layout.rs`.

## Idempotence and Recovery

Every milestone is an independent, atomic change. If a milestone fails tests:

1. Revert the milestone's changes with `git checkout -- .` from `agent/`.
2. Re-read the affected source files to verify current state.
3. Retry the milestone with corrections.

Milestones 1-4 are purely mechanical (delete/rename/relabel) with no behavioral change — the risk of breaking anything is near zero. Milestones 5-7 introduce new code (macros, helpers) that must be behavior-preserving; running tests after each is the primary safety net.

For Milestone 7 (TD-005, the macro), if the macro design proves too complex or doesn't work cleanly with the generated API types, fall back to keeping the manual impls and closing TD-005 as "deferred — macro complexity outweighs benefit." The other 6 items are independent.
