# Update `Outcome` consumers (`main.rs` and integration tests) for `device_name` field

This ExecPlan is a living document. The sections **Progress**, **Surprises & Discoveries**, **Decision Log**, and **Outcomes & Retrospective** must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (this repo, branch `refactor/provision-outcome`, base `feat/reprovision`, PR #52, `mode:push`) | read-write | Adapt `agent/src/main.rs` and `agent/tests/provisioning/provision.rs` to the new `Outcome { is_provisioned, device_name }` shape and the new short-circuit-always-on-keys-present contract. No new error variants; no changes to `provision()` or `reprovision()`. |

This plan lives in `plans/backlog/` of the agent repo. The orchestrator promotes it to `plans/active/` when work begins.

## Purpose / Big Picture

`Outcome` lost its `device: backend_client::Device` field and gained a `device_name: String` field instead. The short-circuit branch in `provision()` now reads `device.json` and projects to just the name (with an `"unknown"` fallback) and — critically — no longer falls through to the full provisioning flow when `device.json` is corrupt: once `assert_activated` succeeds, `provision()` always short-circuits with `is_provisioned: true`. This plan adapts the two downstream consumers (`agent/src/main.rs` and the `provision_fn::*` integration tests) to that new shape and rewrites the one test whose contract changed.

## Progress

- [ ] (YYYY-MM-DD) M1: Update `agent/src/main.rs` `handle_provision_result` to read `outcome.device_name`.
- [ ] (YYYY-MM-DD) M2: Update `agent/tests/provisioning/provision.rs` `provision_fn::*` tests for the new field shape; rewrite the corrupt-device-file test to match the new contract.
- [ ] (YYYY-MM-DD) M3: Confirm `agent/tests/provisioning/reprovision.rs` is unchanged (`reprovision()` returns `Result<backend_client::Device, ProvisionErr>`, not `Outcome`).
- [ ] (YYYY-MM-DD) M4: Confirm `agent/src/provisioning/provision.rs::tests` is unchanged (only tests `determine_settings`).
- [ ] (YYYY-MM-DD) M5: Coverage check — if `agent/src/provisioning/.covgate` (95.66) trips on the unhit `Err(e)` arm in the short-circuit, add a test exercising it.
- [ ] (YYYY-MM-DD) M6: Validation — `./scripts/preflight.sh` reports `Preflight clean`; commit and push to update PR #52.

Use timestamps when you complete steps. Split partially completed work into "done" and "remaining" as needed.

## Surprises & Discoveries

(Fill in as discoveries are made.)

## Decision Log

(Fill in as decisions are made.)

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

### Key files

- `agent/src/provisioning/provision.rs` — defines `Outcome` (already in the new shape on the working tree, uncommitted). Out of scope for edits.
- `agent/src/main.rs` — `handle_provision_result` formats the success messages using `outcome.device.name` today; needs to read `outcome.device_name`.
- `agent/tests/provisioning/provision.rs` — `provision_fn::*` integration tests bind `outcome.device` and read `device.id` / `device.name`; need projection updates plus one rewrite.
- `agent/tests/provisioning/reprovision.rs` — `reprovision()` returns `backend_client::Device` directly, not `Outcome`; expected to be untouched. Confirm by inspection.
- `agent/src/provisioning/.covgate` — 95.66 (line-coverage floor for `agent/src/provisioning/`). Pre-change actual was 96.27%. Don't lower.

### Repo conventions

- **Imports**: three groups labelled `// standard crates`, `// internal crates`, `// external crates`, separated by blank lines.
- **Errors**: every error type derives `thiserror::Error` and implements `crate::errors::Error`. **No new error variants in this plan.**
- **Test gating**: integration tests live in `agent/tests/`, organized by module. Inline unit tests in `#[cfg(test)] mod tests { ... }` blocks alongside the function under test.
- **Workflow**: `./scripts/preflight.sh` runs lint + tests + tools-lint + tools-tests in parallel and prints `Preflight clean` on success.

### Glossary

- **Short-circuit**: the early-return branch in `provision()` taken when `storage::assert_activated(layout)` succeeds. After the user's change it always returns `Outcome { is_provisioned: true, device_name }`, where `device_name` comes from `device.json` if readable and is `"unknown"` otherwise. There is no fall-through to the full flow on read failure.
- **Full flow**: keypair generation, backend POST via `provision_with_backend`, then `storage::setup::bootstrap`. Returns `Outcome { is_provisioned: false, device_name: device.name }` from the freshly-issued backend `Device`.

## M1 — Update `agent/src/main.rs`

**Goal**: replace the two `&outcome.device.name` references with `&outcome.device_name`.

**Files touched**:
- `agent/src/main.rs`

**Code shape**: inside `handle_provision_result`, both the `Ok(outcome) if outcome.is_provisioned => { ... }` arm and the bare `Ok(outcome) => { ... }` arm currently format with `display::color(&outcome.device.name, display::Colors::Green)`. Replace `&outcome.device.name` with `&outcome.device_name` in both arms. The `Err(e)` arm is unchanged.

**Test additions**: none. `main.rs` has no unit tests of `handle_provision_result`; integration coverage comes from M2.

**Validation step**: `cargo build -p miru-agent` and `cargo build -p miru-agent --features test` both succeed.

## M2 — Update `agent/tests/provisioning/provision.rs`

**Goal**: project from `outcome.device.{id,name}` to `outcome.device_name` in every `provision_fn::*` test that binds the field, and rewrite the corrupt-device-file test to the new contract.

**Files touched**:
- `agent/tests/provisioning/provision.rs`

**Per-test updates**:

| Test (`provision_fn::*`) | Change |
|---|---|
| `success` | drop `let device = outcome.device;`; drop `assert_eq!(device.id, DEVICE_ID);`; replace `assert_eq!(device.name, device_name);` with `assert_eq!(outcome.device_name, device_name);`. |
| `http_error_aborts_provision` | unchanged (returns `Err`, no `outcome.device` access). |
| `provision_is_idempotent_on_second_call` | drop `let device = outcome.device;` and `assert_eq!(device.id, DEVICE_ID);`; replace `assert_eq!(device.name, "first");` with `assert_eq!(outcome.device_name, "first");`. |
| `is_noop_when_already_activated` | drop `let device = outcome.device;` and `assert_eq!(device.id, DEVICE_ID);`. Add `assert_eq!(outcome.device_name, "initial");` to preserve semantic intent. |
| `falls_through_when_keys_missing` | drop `let device = outcome.device;`; replace `assert_eq!(device.name, "after-fallthrough");` with `assert_eq!(outcome.device_name, "after-fallthrough");`. Still works — keys are absent so the short-circuit fails and the full flow runs. |
| `falls_through_when_device_file_corrupt` | **Rewrite** (see code shape below). Rename to `is_noop_when_device_file_corrupt`. |
| `http_error_on_reprovision_preserves_existing_storage` | unchanged (no `outcome.device` access; only reads `outcome.is_provisioned`). |

**Code shape — rewritten `is_noop_when_device_file_corrupt`** (rename target for `falls_through_when_device_file_corrupt`):

- Setup is identical: first provision lays down keys + a valid `device.json`.
- Then overwrite `device.json` with `"not valid json{"` via `WriteOptions::OVERWRITE_ATOMIC` (same as before).
- Capture `let device_bytes = layout.device().read_string().await.unwrap();` immediately after the corruption write so the post-call comparison is byte-exact.
- Call `provision()` a second time with a `MockClient` whose `provision_device_fn` would `Ok(new_device(DEVICE_ID, "recovered"))` if invoked.
- Assert `outcome.is_provisioned == true` (new contract — no fall-through).
- Assert `outcome.device_name == "unknown"` (the `Err(e)` arm of the `read_json::<models::Device>()` call returns the `"unknown"` fallback).
- Assert `mock.call_count(mock::Call::ProvisionDevice) == 0` (short-circuit means the backend mock is never reached).
- Assert `layout.device().read_string().await.unwrap() == device_bytes` — the corrupt bytes are NOT overwritten, because the full flow (which would have called `bootstrap`) is never run.
- Assert keys are still present (`auth_layout.private_key().exists()`, `auth_layout.public_key().exists()`).
- `root.delete().await.unwrap();` at the end as in every sibling.

**Test additions**: the rewrite above counts as the only added/changed test body in M2. No additional tests yet; M5 may add one for coverage.

**Validation step**: `cargo test -p miru-agent --features test provisioning::provision::provision_fn` passes all seven tests.

## M3 — Confirm `agent/tests/provisioning/reprovision.rs` is unchanged

**Goal**: verify `reprovision_fn::*` does not touch `Outcome`.

**Files touched**: none.

**Code shape**: `reprovision::reprovision(...).await` returns `Result<backend_client::Device, ProvisionErr>` (its own struct, not `Outcome`). A quick grep confirms no `outcome.device` references in this file. If any are found, that's a bleed-over and should be flagged in Surprises before the implementer makes changes.

**Validation step**: `grep -n "outcome\.device\|provision::Outcome" agent/tests/provisioning/reprovision.rs` returns nothing.

## M4 — Confirm source unit tests are unchanged

**Goal**: verify `agent/src/provisioning/provision.rs::tests` only covers `determine_settings`.

**Files touched**: none.

**Code shape**: the `#[cfg(test)] mod tests` block at the bottom of `agent/src/provisioning/provision.rs` contains exactly one inner module `mod determine_settings { ... }` exercising `cli::ProvisionArgs` -> `settings::Settings`. None of the cases construct or destructure `Outcome`. No edits required.

**Validation step**: `grep -n "Outcome\|outcome\." agent/src/provisioning/provision.rs` shows hits only in the `Outcome` definition, the function returns/bodies, and (post-edit) nothing else.

## M5 — Coverage check

**Goal**: keep coverage at or above the `.covgate` threshold (95.66 for `agent/src/provisioning/`).

**Files possibly touched**:
- `agent/tests/provisioning/provision.rs` (one new test if needed)

**Code shape — only if covgate fails**: the new short-circuit `Err(e) => { error!(...); "unknown".to_string() }` arm in `provision()` is exercised by the rewritten `is_noop_when_device_file_corrupt` test (M2), since that test corrupts `device.json` and re-runs `provision()`. If for some reason that test does not register against the covgate metric, add:

```rust
#[tokio::test]
async fn short_circuits_with_unknown_when_device_file_missing() {
    // signature only — body to be filled by implementer
}
```

The body should leave the layout in a state where `assert_activated` succeeds (auth keys + token present) but `device.json` is absent or unreadable — easiest path is to run a successful provision, capture state, then `layout.device().delete().await.unwrap();` before the second call. Assert `outcome.is_provisioned == true`, `outcome.device_name == "unknown"`, and the mock's `ProvisionDevice` call count is 0.

**Validation step**: re-run `./scripts/preflight.sh`; covgate output line for `agent/src/provisioning/` is at or above 95.66.

## M6 — Validation

**Goal**: clean preflight, then commit and push.

Run from the repo root:

1. `cargo build -p miru-agent --features test` — clean.
2. `cargo build -p miru-agent` — clean.
3. `cargo test -p miru-agent --features test provisioning::` — every `provision_fn::*` (seven, with the renamed test) and `reprovision_fn::*` integration case passes.
4. `./scripts/preflight.sh` — final line reads `Preflight clean`.

> **Validation gate**: `./scripts/preflight.sh` MUST report `Preflight clean` before the changes are pushed.

If preflight fails:

- **Compile errors in `main.rs`**: confirm both arms of `handle_provision_result` use `&outcome.device_name` and that nothing else still references `outcome.device`.
- **Compile errors in `provision.rs` tests**: search for any remaining `outcome.device` or `let device = outcome.device;` and remove them.
- **Test failure in `is_noop_when_device_file_corrupt`**: most likely the `device_bytes` snapshot was taken before (not after) the corruption write, or the assertion compared post-call bytes to a fresh JSON instead of the corrupt blob.
- **Covgate failure**: see M5 — add the `short_circuits_with_unknown_when_device_file_missing` test.
- **Lint failures**: standard lint (unused-import, unused-let, formatting). The removed `let device = outcome.device;` lines should be deleted entirely, not left as `let _ = ...`.

Re-run `./scripts/preflight.sh` until the final line is `Preflight clean`. Then `git add agent/src/provisioning/provision.rs agent/src/main.rs agent/tests/provisioning/provision.rs` (the user's working-tree change to `provision.rs` lands in the same commit as the consumer adaptations) and create a single commit on `refactor/provision-outcome`. `git push` to update PR #52.

## Validation and Acceptance

The change is accepted when ALL of the following hold:

1. `./scripts/preflight.sh` exits 0 and prints `Preflight clean`. **This gate is non-negotiable.**
2. `agent/src/main.rs` `handle_provision_result` references `outcome.device_name` in both success arms; no `outcome.device` reference remains.
3. `grep -n "outcome\.device\b\|let device = outcome\.device" agent/tests/provisioning/provision.rs` returns no matches.
4. `provision_fn::is_noop_when_device_file_corrupt` exists (renamed from `falls_through_when_device_file_corrupt`) and asserts `is_provisioned == true`, `device_name == "unknown"`, mock call count == 0, and that the corrupt bytes were not overwritten.
5. `agent/src/provisioning/.covgate` is unchanged at 95.66 and `agent/src/provisioning/` line coverage is at or above that floor.
6. No edits land in `agent/src/provisioning/provision.rs` beyond the user's already-uncommitted `Outcome` shape change, and no edits land in `agent/tests/provisioning/reprovision.rs`, `agent/src/provisioning/reprovision.rs`, or `agent/src/provisioning/shared.rs`.
7. PR #52 on branch `refactor/provision-outcome` is updated with the resulting commit. No new branch, no new PR.

## Idempotence and Recovery

- All edits are local rewrites of small text regions; rerunning a milestone yields identical content.
- If a milestone aborts mid-way, `git status` shows the partial state and `git restore -SW agent/src/main.rs agent/tests/provisioning/provision.rs` reverts cleanly. (Don't `git restore` `agent/src/provisioning/provision.rs` — that's the user's uncommitted shape change.)
- `mode:push`: branch `refactor/provision-outcome` is already on origin attached to PR #52 (base `feat/reprovision`). After preflight is clean, commit the Outcome shape change plus the consumer adaptations as a single commit and push to update the PR. No new branch, no new PR.
