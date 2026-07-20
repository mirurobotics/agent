# Remove dead error types and gate test-only scan accessors

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (repo root of mirurobotics/agent) | read-write | Delete never-referenced error types and a type alias from `agent/src/disk/`, `agent/src/server/`, and `agent/src/provisioning/`; gate three test-only accessors in `agent/src/scan/` behind `#[cfg(feature = "test")]`. |

This plan lives in `plans/` of the agent repo because all changes are inside this repo. Work happens on branch `claude/agnet-deadcode-cleanup-ebr5oq`, created from `main` at `7b15b90`.

## Purpose / Big Picture

A production build of the agent (`cargo check --workspace`, no features) currently emits two `dead_code` warnings from the scan module, and the error modules carry five error types that are never constructed anywhere in any build configuration. Dead error variants inflate the error enums, mislead readers about which failure paths actually exist, and the warnings train people to ignore compiler output. After this change, `cargo check --workspace` is warning-free, the error enums contain only variants that are actually produced, and code whose only callers are the feature-gated test/query API is explicitly marked as such via the repo's `#[cfg(feature = "test")]` convention.

No runtime behavior changes: everything removed is unreachable, and everything gated is only reachable from test builds.

## Progress

- [ ] Milestone 1: delete dead code, gate test-only accessors, commit.
- [ ] Milestone 2: local validation (check, test, lint) and CI green on the pushed head.

## Surprises & Discoveries

(Add entries as you go.)

## Decision Log

- Decision: Gate `CollectionScanner::rule`, `CollectionScanner::ledger_count`, and `CollectionState::ledger_count` behind `#[cfg(feature = "test")]` rather than delete them.
  Rationale: they are live code — their callers are the feature-gated scanner query API and unit tests — so deletion would break test builds; gating matches the repo's existing convention for test-only code (e.g. `get_rules` / `get_ledger_count` in `agent/src/scan/scanner.rs`) and silences the production `dead_code` warnings honestly. Date/Author: 2026-07-20 / agents@miruml.com.
- Decision: Delete `MissingDeviceIDErr` outright instead of wiring it up.
  Rationale: it is a leftover twin of `disk::ResolveDeviceIDErr` (`agent/src/disk/errors.rs`) from before device-id resolution moved into the disk module; the disk version is the one actually constructed, so the server copy is pure residue. Date/Author: 2026-07-20 / agents@miruml.com.

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

Repo root: the checkout of `mirurobotics/agent` (Rust workspace; the binary crate is `miru-agent` under `agent/`). All paths are repo-root-relative; all commands run from the repo root. Read `AGENTS.md` first: `./scripts/test.sh` wraps `RUST_LOG=off cargo test --features test` (never run bare `cargo test`); lint via `./scripts/lint.sh`. The `test` cargo feature gates test-only helpers so they exist in test builds but not in the shipped binary.

Error-module pattern: each module (e.g. `agent/src/server/errors.rs`) defines concrete error structs, wraps them in a module-level enum (`ServerErr`, `DiskErr`, `ProvisionErr`), and registers each variant with the `crate::impl_error!` macro. Removing an error type therefore means removing three things together: the struct (plus its `impl crate::errors::Error`), the enum variant, and the `impl_error!` entry — and any `From` impl or test that exists only for that type.

The dead code, verified never referenced in any build configuration (`rg` the identifier; only definition sites appear):

- `CfgInstEntry` — type alias in `agent/src/disk/config_instances.rs`, never used.
- `MissingDeviceIDErr` — `agent/src/server/errors.rs`; leftover twin of `disk::ResolveDeviceIDErr` (see Decision Log). Struct + `ServerErr` variant + `impl_error!` entry.
- `SendShutdownSignalErr` — `agent/src/server/errors.rs`; never constructed. Struct + variant + `impl_error!` entry.
- `InvalidSettingsErr` — `agent/src/provisioning/errors.rs`; never constructed outside its own `From`-impl unit test (`from_invalid_settings_err`), which tests nothing but the type's own plumbing. Struct + variant + `From` impl + `impl_error!` entry + that test.
- `PruneCacheErrs` — `agent/src/disk/errors.rs`; never constructed. Struct + variant + `impl_error!` entry.

The warning sources: `CollectionScanner::rule` and `CollectionScanner::ledger_count` (`agent/src/scan/collection.rs`) and `CollectionState::ledger_count` (`agent/src/scan/state.rs`) are only called from the `#[cfg(feature = "test")]` scanner query API and from unit tests, so a no-features build reports them as dead (`cargo check --workspace` shows 2 `dead_code` warnings today).

Environment quirk: in this container `./scripts/test.sh` has 17 known pre-existing failures — root-environment permission-test artifacts, identical on the base branch (`7b15b90`). Any failure beyond those 17 is a regression.

## Plan of Work

Single milestone, all edits together (they are independent deletions with no ordering constraints):

1. `agent/src/disk/config_instances.rs`: delete the `CfgInstEntry` type alias.
2. `agent/src/server/errors.rs`: delete the `MissingDeviceIDErr` struct (with its manual `Display` impl and `Error` impl), the `SendShutdownSignalErr` struct (with its `Error` impl), their `ServerErr` variants, and their `impl_error!` entries.
3. `agent/src/provisioning/errors.rs`: delete the `InvalidSettingsErr` struct (with its `Error` impl), the `ProvisionErr::InvalidSettingsErr` variant, the `impl From<InvalidSettingsErr> for ProvisionErr` impl, the `impl_error!` entry, and the `from_invalid_settings_err` unit test.
4. `agent/src/disk/errors.rs`: delete the `PruneCacheErrs` struct (with its `Error` impl), the `DiskErr::PruneCacheErrs` variant, and the `impl_error!` entry.
5. `agent/src/scan/collection.rs`: add `#[cfg(feature = "test")]` to `CollectionScanner::rule` and `CollectionScanner::ledger_count`. Since `UploadRule` is then only named inside the gated function, drop it from the top-level `use crate::models::{...}` import and reference it as `crate::models::UploadRule` in the gated signature (avoids an unused-import warning in production builds).
6. `agent/src/scan/state.rs`: add `#[cfg(feature = "test")]` to `CollectionState::ledger_count`.
7. Commit: `refactor: remove dead error types and gate test-only scan accessors`.

No new tests are added: the change removes unreachable code and one self-referential test; the acceptance signal is a warning-free production check plus the existing suite staying green in both feature configurations.

## Concrete Steps

All commands run from the repo root on branch `claude/agnet-deadcode-cleanup-ebr5oq`.

Milestone 1:

    # edit the six files per Plan of Work
    cargo check --workspace                                   # expect: zero warnings
    cargo check --workspace --all-targets --features test     # expect: compiles cleanly
    cargo fmt -p miru-agent
    git add -A && git commit -m "refactor: remove dead error types and gate test-only scan accessors"

Milestone 2:

    ./scripts/test.sh    # expect: no failures beyond the 17 known root-permission artifacts
    ./scripts/lint.sh    # expect: passes end-to-end
    git push origin claude/agnet-deadcode-cleanup-ebr5oq

Then watch the CI workflow on the pushed head until green. Milestone 2 produces no commit unless fixes are needed.

## Validation and Acceptance

1. `cargo check --workspace` completes with zero warnings (before the change it reports 2 `dead_code` warnings from `agent/src/scan/`).
2. `cargo check --workspace --all-targets --features test` compiles cleanly — proof the gated accessors and their test/query-API callers still line up.
3. `./scripts/test.sh` passes with no new failures. The 17 known failures in this container are root-environment permission-test artifacts and are identical on the base branch (`7b15b90`); verify any failure against that baseline before attributing it to this change.
4. `./scripts/lint.sh` passes end-to-end.
5. Preflight reports CLEAN: CI is green on the pushed head of `claude/agnet-deadcode-cleanup-ebr5oq`. The PR must not leave draft, and the task must not be reported complete, until CI is green on that head.

## Idempotence and Recovery

All steps are safe to re-run: the edits are convergent deletions/annotations, and `cargo check`, `test.sh`, and `lint.sh` are read-only. Before the commit, `git checkout -- <file>` reverts a bad edit; the branch exists only for this work, so `git reset --hard 7b15b90` is a full rollback pre-push, and a force-push of the reset branch recovers post-push. If a supposedly-dead symbol turns out to be referenced after all, the compiler fails immediately at the `cargo check` steps and the offending deletion can be reverted in isolation — each deletion is independent of the others.
