# Refactor `provision()` to return `ProvisionOutcome` so callers can distinguish no-op from fresh provision

This ExecPlan is a living document. The sections **Progress**, **Surprises & Discoveries**, **Decision Log**, and **Outcomes & Retrospective** must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (this repo, branch `refactor/provision-outcome`, base `feat/reprovision`) | read-write | Source, tests, CLI binary, and plan changes for this refactor. |
| `libs/backend-api/` (inside this repo, generated) | read-only | Generated client types are consumed only via `backend_api::models as backend_client` — do not edit. |

This plan lives in `plans/backlog/` of the agent repo because every code change is in this repo.

The branch `refactor/provision-outcome` was freshly cut from `feat/reprovision` (PR #50, not yet merged) and has no commits on top yet. The base branch `feat/reprovision` already contains:

- The `reprovision` command and HTTP wrapper.
- The current `provision()` short-circuit that returns `Result<backend_client::Device, ProvisionErr>` and reads `device.json` to populate the returned `Device`.
- The full integration test suite under `agent/tests/provision/entry.rs` with `provision_fn::*` and `reprovision_fn::*` modules.
- A shared `cleanup_temp_dir()` helper in `agent/src/provision/entry.rs`.

## Purpose / Big Picture

Today `provision()` returns a `Result<backend_client::Device, ProvisionErr>` whether the call did real work or short-circuited because the box was already activated. The caller in `main.rs` then unconditionally prints `"Successfully provisioned this device as <name>!"` — which is misleading on the no-op branch where nothing was provisioned.

This refactor introduces a thin wrapper:

```rust
pub struct ProvisionOutcome {
    pub is_provisioned: bool,
    pub device: backend_client::Device,
}
```

Naming convention (load-bearing — document on the struct's doc comment): `is_provisioned == true` means **the device was already provisioned before this call**; the call was a no-op and `device` is the cached state read from `device.json`. `is_provisioned == false` means **this call performed the full provisioning flow** (keypair gen, backend POST, bootstrap) and `device` is the freshly-issued backend record.

The caller branches on `outcome.is_provisioned` to render either:

- `is_provisioned: true`  -> `"Device is already provisioned as <name>!"`
- `is_provisioned: false` -> `"Successfully provisioned this device as <name>!"`

The refactor also makes the comment on the short-circuit honest: we read `device.json` because we need a name for the outcome's `device` field, not because falling through is a real recovery path. (If keys are on disk, the backend has already registered them; re-running `provision` with new keys would at best create a duplicate or fail.)

`reprovision()` is intentionally untouched. It has only one path (always rotates keys, always calls the backend) so its `Result<backend_client::Device, ProvisionErr>` signature already conveys everything the caller needs.

A developer verifies success by:

1. Running `./scripts/preflight.sh` and seeing `Preflight clean`.
2. Running `cargo test -p miru-agent --features test provision::` and seeing all `provision_fn::*` and `reprovision_fn::*` cases pass — including the now-augmented `is_provisioned` assertions in `provision_fn::*`.
3. Spot-check: `grep -n "ProvisionOutcome" agent/src/provision/entry.rs agent/src/main.rs` shows the struct definition, the `provision()` return type, and the two-branch match in `handle_provision_result`.

User-visible behavior change is purely the wording on the no-op branch; the full-provision message is unchanged, the error path is unchanged, and `reprovision` output is unchanged.

## Progress

- [ ] (YYYY-MM-DD) M1: Add `ProvisionOutcome` struct in `agent/src/provision/entry.rs`.
- [ ] (YYYY-MM-DD) M2: Change `provision()` return type to `Result<ProvisionOutcome, ProvisionErr>` and rewrite the short-circuit comment.
- [ ] (YYYY-MM-DD) M3: Confirm `reprovision()` is unchanged (no edits).
- [ ] (YYYY-MM-DD) M4: Update `main.rs` `handle_provision_result` to branch on `outcome.is_provisioned`.
- [ ] (YYYY-MM-DD) M5: Update integration tests in `agent/tests/provision/entry.rs` (`provision_fn::*`).
- [ ] (YYYY-MM-DD) M6: Coverage check (`agent/src/provision/.covgate`); add coverage if dropped, never lower the threshold.
- [ ] (YYYY-MM-DD) M7: Validation — `./scripts/preflight.sh` reports `Preflight clean`.

Use timestamps when you complete steps. Split partially completed work into "done" and "remaining" as needed.

## Surprises & Discoveries

(Capture anything the implementation reveals that the plan did not anticipate.)

## Decision Log

(Record non-obvious choices made during implementation.)

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

### Repo conventions

- **Imports**: every source file uses three groups separated by blank lines and labelled with a comment: `// standard crates`, `// internal crates`, `// external crates`. The lint runner enforces this. See `agent/src/provision/entry.rs` for a canonical example.
- **Errors**: every error type derives `thiserror::Error` and implements `crate::errors::Error`; aggregating enums use the `crate::impl_error!` macro. **No new error variants in this plan.**
- **Test feature gate**: helpers and mocks are gated behind `#[cfg(feature = "test")]`. Unit tests live inside `#[cfg(test)] mod tests { ... }` in the same source file as the code under test; integration tests live under `agent/tests/<module>/...` and are reached through `RUST_LOG=off cargo test --features test`.
- **Workflow**:
  - `./scripts/test.sh` runs the agent test suite with the `test` feature on.
  - `./scripts/lint.sh` runs the custom import linter, `cargo fmt`, machete, audit, and clippy.
  - `./scripts/preflight.sh` runs lint + tests + tools-lint + tools-tests in parallel and prints `Preflight clean` on success.
- **Coverage gate** for this plan: `agent/src/provision/.covgate` is currently `95.66`. PR #50 raised real coverage to roughly 96.27%. Re-read the file before the work concludes and act on the actual value. Do **not** lower the threshold.

### Key files

- `agent/src/provision/entry.rs` — owns `provision`, `reprovision`, `provision_with_backend`, `reprovision_with_backend`, the shared `cleanup_temp_dir`, `read_token_from_env`, and the `build_settings` / `determine_settings` / `determine_reprovision_settings` triple. The new `ProvisionOutcome` struct lives at the top of this file.
- `agent/src/provision/mod.rs` — re-exports `entry::*` and `errors::ProvisionErr`. The wildcard `pub use self::entry::*;` already makes `ProvisionOutcome` reachable as `provision::ProvisionOutcome` without any change to `mod.rs`. Verify by running `grep -n "pub use" agent/src/provision/mod.rs` before editing.
- `agent/src/main.rs` — defines `run_provision`, `handle_provision_result`, `run_reprovision`, `handle_reprovision_result`. Only `handle_provision_result`'s match arms change here; `run_provision` already returns the same type as `provision::provision`, so its return type updates mechanically.
- `agent/tests/provision/entry.rs` — integration tests for `provision_fn::*` and `reprovision_fn::*`. All call-sites of `provision::provision(...).await.unwrap()` in the `provision_fn::*` module need to unwrap to a `ProvisionOutcome` and then project to `device`.

### Glossary

- **`ProvisionOutcome`**: the new wrapper return type; `is_provisioned` is `true` iff the call was a no-op short-circuit, `false` iff the call performed the full flow.
- **No-op short-circuit**: the branch in `provision()` taken when `storage::assert_activated(layout)` succeeds AND `layout.device().read_json::<models::Device>()` parses. The cached local `Device` is projected onto a default `backend_client::Device` and returned.
- **Full provisioning flow**: keypair gen in temp dir -> backend POST `/devices/provision` -> `storage::setup::bootstrap` -> temp-dir cleanup. Always returns the freshly-issued backend `Device`.

## M1 — Add `ProvisionOutcome` struct

**Goal**: add a `pub struct ProvisionOutcome` at the top of `agent/src/provision/entry.rs` (after the imports, before `pub async fn provision`).

**Files touched**:
- `agent/src/provision/entry.rs`

**Code shape** (signature and doc comment only — no constructor):

```rust
/// The result of a `provision()` call.
///
/// `is_provisioned` is `true` when the machine was already provisioned
/// before this call — i.e., the call was a no-op and `device` is the
/// cached state read from `device.json`. It is `false` when this call
/// performed the full provisioning flow (keypair gen, backend POST,
/// bootstrap), in which case `device` is the freshly-issued backend
/// record.
pub struct ProvisionOutcome {
    pub is_provisioned: bool,
    pub device: backend_client::Device,
}
```

Both call sites construct `ProvisionOutcome` directly with field syntax — there is no constructor function to write.

**Test additions**: none for this milestone. The struct has no logic; integration tests in M5 cover both fields on every code path.

**Validation step**: `cargo build -p miru-agent --features test` — clean build with the new struct in place but `provision()` still returning the old type (it should still compile because nothing references the new struct yet). It is acceptable for this milestone to leave `ProvisionOutcome` unused; the unused-struct lint is `#[allow]`-worthy if it fires.

## M2 — Change `provision()` return type to `Result<ProvisionOutcome, ProvisionErr>`

**Goal**: update the function signature, the short-circuit return, the success return at the bottom, and the comment on the short-circuit. Keep the body's logic identical (keypair gen, backend POST, bootstrap, temp-dir cleanup are all preserved).

**Files touched**:
- `agent/src/provision/entry.rs`

**Code shape** (signature and the two return-site shapes only — full body is unchanged):

```rust
pub async fn provision<HTTPClientT: http::ClientI>(
    http_client: &HTTPClientT,
    layout: &storage::Layout,
    settings: &settings::Settings,
    token: &str,
    device_name: Option<String>,
) -> Result<ProvisionOutcome, ProvisionErr> {
    // Idempotency short-circuit: if the machine is already activated and
    // device.json is parseable, return the cached device with `is_provisioned`
    // set so the caller can render an "already provisioned" message. We need
    // device.json to populate the outcome's device field. If it's missing
    // despite keys being present, the bootstrap was interrupted mid-way; fall
    // through and let the backend tell us whether re-provisioning is possible.
    if storage::assert_activated(layout).await.is_ok() {
        if let Ok(local_device) = layout.device().read_json::<models::Device>().await {
            return Ok(ProvisionOutcome {
                is_provisioned: true,
                device: backend_client::Device {
                    id: local_device.id,
                    name: local_device.name,
                    session_id: local_device.session_id,
                    ..backend_client::Device::default()
                },
            });
        }
    }

    // ...existing temp_dir / gen_key_pair / provision_with_backend / bootstrap
    //    logic unchanged...

    // success return at the bottom of the inner async block:
    Ok(ProvisionOutcome { is_provisioned: false, device })
    // (the `cleanup_temp_dir(&temp_dir).await; result` tail also unchanged)
}
```

The replacement comment block above is the **honest** version — preserve it verbatim. Do not retain wording about "fall through to the full provisioning flow so the box can recover": with keys already on disk the backend has already registered them, so falling through is best-effort, not a real recovery path.

`reprovision()`, `provision_with_backend()`, `reprovision_with_backend()`, `cleanup_temp_dir()`, `read_token_from_env()`, `build_settings()`, `determine_settings()`, and `determine_reprovision_settings()` are all untouched.

**Test additions**: covered in M5 (the call sites' assertions change).

**Validation step**: `cargo build -p miru-agent --features test` will fail at all `provision::provision(...).await` callers (in `main.rs` and the integration tests). That is expected — those are fixed in M4 and M5. Build clean is **not** the bar for this milestone in isolation.

## M3 — `reprovision()` is unchanged

**Explicit non-goal**: `reprovision()` continues to return `Result<backend_client::Device, ProvisionErr>`. Reprovision has only one path — full flow, always — so wrapping it in a `ProvisionOutcome` would add a field whose value is constant. Out of scope for this refactor.

**Files touched**: none.

**Validation step**: `grep -n "pub async fn reprovision" agent/src/provision/entry.rs` shows the signature unchanged with `Result<backend_client::Device, ProvisionErr>`.

## M4 — Update `main.rs` `handle_provision_result`

**Goal**: branch on `outcome.is_provisioned` to print one of two messages. The error branch is unchanged. The `run_provision` function's return type updates mechanically (it already returns whatever `provision::provision` returns).

**Files touched**:
- `agent/src/main.rs`

**Code shape**:

```rust
async fn run_provision(args: cli::ProvisionArgs) -> Result<provision::ProvisionOutcome, ProvisionErr> {
    // body unchanged — returns the value from provision::provision(...).await
}

fn handle_provision_result(result: Result<provision::ProvisionOutcome, ProvisionErr>) {
    match result {
        Ok(outcome) if outcome.is_provisioned => {
            let msg = format!(
                "Device is already provisioned as {}!",
                display::color(&outcome.device.name, display::Colors::Green)
            );
            println!("{}", display::format_info(msg.as_str()));
        }
        Ok(outcome) => {
            let msg = format!(
                "Successfully provisioned this device as {}!",
                display::color(&outcome.device.name, display::Colors::Green)
            );
            println!("{}", display::format_info(msg.as_str()));
        }
        Err(e) => {
            error!("Provisioning failed: {:?}", e);
            println!("An error occurred during provisioning. Contact us at ben@mirurobotics.com for immediate support.\n\nError: {e}\n");
            std::process::exit(1);
        }
    }
}
```

Import note: `provision::ProvisionOutcome` is reachable via the existing wildcard re-export from `agent/src/provision/mod.rs` (`pub use self::entry::*;`). No change to `mod.rs`. Verify with `grep -n "pub use" agent/src/provision/mod.rs` before editing — if the wildcard is still in place, the import path works.

If `main.rs` references the type directly (e.g., the `run_provision` return type), keep the qualified `provision::ProvisionOutcome` form rather than adding a new `use` line. The existing `use miru_agent::provision::{self, display, errors::*};` already exposes the path.

`run_reprovision` and `handle_reprovision_result` are unchanged. The error message in `handle_provision_result` is unchanged.

**Test additions**: none. `main.rs` has no direct test coverage in this repo (it's the binary entry).

**Validation step**: `cargo build -p miru-agent --features test` and `cargo build -p miru-agent` (without `test`) — both clean.

## M5 — Update integration tests in `agent/tests/provision/entry.rs`

**Goal**: every `provision_fn::*` test that does `let device = provision::provision(...).await.unwrap();` becomes:

```rust
let outcome = provision::provision(...).await.unwrap();
assert!(outcome.is_provisioned == <expected>);
let device = outcome.device;
// rest of assertions unchanged
```

The tests as currently named in `agent/tests/provision/entry.rs` (verified by `grep -n "async fn" agent/tests/provision/entry.rs`):

| Test (`provision_fn::*`) | Expected `is_provisioned` |
|---|---|
| `success` | `false` (full flow on a fresh tempdir) |
| `http_error_aborts_provision` | n/a — returns `Err`; no outcome to inspect |
| `provision_is_idempotent_on_second_call` | first call: `false`; second call: `true` |
| `is_noop_when_already_activated` | first call: `false`; second call: `true` |
| `falls_through_when_keys_missing` | `false` (keys absent -> short-circuit fails -> full flow) |
| `falls_through_when_device_file_corrupt` | first call: `false`; second call (after corrupting `device.json`): `false` (parse fails -> full flow) |
| `http_error_on_reprovision_preserves_existing_storage` | first call: `false`; second call: `true` (per the existing test body, the second call hits the no-op path because keys + parseable `device.json` are still on disk; the failing mock is never invoked) |

Notes:

- Tests that currently call `.unwrap()` and **don't** bind a name to the result (e.g., the seed call in `provision_is_idempotent_on_second_call` written as `provision::provision(...).await.unwrap();`) only need to compile after the type change; an `assert!(!outcome.is_provisioned)` on the seed call is optional but recommended for documentation, and matches the table above.
- The `reprovision_fn::*` tests are unchanged — `reprovision()` still returns `Result<backend_client::Device, ProvisionErr>`.
- The `http_error_aborts_provision` test only matches `Err(ProvisionErr::HTTPErr(_))`; that match still compiles because the error variant of the new `Result<ProvisionOutcome, ProvisionErr>` is the same `ProvisionErr`.
- For `http_error_on_reprovision_preserves_existing_storage` the existing body already asserts `result.is_ok()` and `mock_fail.call_count(...) == 0`; add an `assert!(outcome.is_provisioned)` after binding the `Ok` value. If that assertion fails, the implementer should re-read the test body — the test description above is grep-confirmed but the assertion semantics are checked by running the test.

**Files touched**:
- `agent/tests/provision/entry.rs`

**Test additions**: no new tests. The struct itself has no logic — its two fields are exercised by every existing path.

**Validation step**: `cargo test -p miru-agent --features test provision::` — every `provision_fn::*` and `reprovision_fn::*` case passes.

## M6 — Coverage check

**Goal**: ensure `agent/src/provision/.covgate` is still satisfied after the refactor.

The relevant covgate threshold is `agent/src/provision/.covgate`. The previous PR #50 raised the actual coverage to roughly 96.27%; the threshold may be `95.66` or higher. Read the file to confirm before acting.

The refactor does not introduce a new branch — the short-circuit `if let Ok(...)` already exists; this plan only changes the shape of the returned value. Coverage may shift by a fraction of a percent. If it drops below the threshold:

- The most likely uncovered fragment is the `ProvisionOutcome { is_provisioned: false, device }` construction path on `success`-style tests vs. the `is_provisioned: true` construction on no-op tests. Both paths are covered by the M5 test updates.
- Add coverage by tightening assertions on existing tests rather than adding new tests. **Do not lower the threshold.**

**Files touched** (only if coverage drops): `agent/tests/provision/entry.rs` to add an assertion on a previously-unasserted field. No `.covgate` file is modified.

**Validation step**: `./scripts/covgate.sh` (or whatever the preflight invokes) reports `agent/src/provision` above its threshold.

## M7 — Validation

Run `./scripts/preflight.sh` from the repo root.

> **Validation gate**: `./scripts/preflight.sh` MUST report `Preflight clean` before changes are pushed or a PR is opened.

If preflight fails:

- **Lint failures** (import order, fmt, clippy): fix per the conventions documented in `agent/AGENTS.md`. The most likely failure is an unused-import warning if `models` becomes unreferenced after the refactor — it should still be referenced by the short-circuit branch (`models::Device`); if not, leave it.
- **Tests**: read the failure. The most likely cause is a missed `outcome.is_provisioned` assertion or a missed `let device = outcome.device;` projection in `provision_fn::*`.
- **Coverage**: see M6; never lower a threshold.
- **Tools lint / tools tests**: typically unaffected by this change. If they fail, investigate whether a generated file under `libs/backend-api/` was inadvertently touched — the v04 regen is already committed and the agent code only consumes existing models here.

Re-run `./scripts/preflight.sh` until the final line is `Preflight clean`.

## Validation and Acceptance

The change is accepted when ALL of the following hold:

1. `./scripts/preflight.sh` exits 0 and prints `Preflight clean`. **This gate is non-negotiable: `./scripts/preflight.sh` MUST report `Preflight clean` before changes are pushed or a PR is opened.**
2. `cargo test -p miru-agent --features test provision::` runs every `provision_fn::*` and `reprovision_fn::*` case and all pass.
3. `grep -n "pub struct ProvisionOutcome" agent/src/provision/entry.rs` shows the struct definition once.
4. `grep -n "Result<ProvisionOutcome" agent/src/provision/entry.rs agent/src/main.rs` shows the new return type at `provision::provision`, `run_provision`, and `handle_provision_result`.
5. `grep -n "is_provisioned" agent/src/provision/entry.rs agent/src/main.rs agent/tests/provision/entry.rs` shows the field set or asserted in: the struct definition, both construction sites in `provision()`, the two-branch match in `handle_provision_result`, and at least one assertion in each `provision_fn::*` test that completes with `Ok(_)`.
6. `grep -n "pub async fn reprovision" agent/src/provision/entry.rs` still shows `Result<backend_client::Device, ProvisionErr>` — the refactor did not leak into `reprovision`.
7. The output messages on the no-op branch read `"Device is already provisioned as <name>!"` and on the full-flow branch read `"Successfully provisioned this device as <name>!"` — both green-colored via `display::color(_, Colors::Green)`.
8. No `.covgate` file is modified.

## Idempotence and Recovery

- All edits are pure source/test changes; rerunning steps re-edits the same file content.
- The `provision()` short-circuit semantics are unchanged: it still triggers only when `assert_activated` succeeds AND `device.json` is parseable. Any partial state still falls through to the full flow, so a half-installed box still recovers via the existing path. Only the **shape of the value returned** changes.
- The new `ProvisionOutcome::is_provisioned` field is purely informational — it does not change which code path executes; the no-op short-circuit still runs the same branches it did before. A bug in the wrapper (e.g., wiring `is_provisioned: true` on the full-flow path) would surface immediately in `provision_fn::success` (which asserts `false`) and `provision_fn::is_noop_when_already_activated` (which asserts `true` on the second call).
- `reprovision()` is unchanged, so all reprovision call sites in `main.rs` and `agent/tests/provision/entry.rs::reprovision_fn::*` continue to compile and pass without edits.
- Branch base note: this branch is stacked on `feat/reprovision` (PR #50). If `feat/reprovision` lands first, this branch retargets to `main` cleanly because the refactor only touches files already modified by PR #50 — there is no new file that depends on a not-yet-merged change elsewhere.
