# Split `agent/src/provision/entry.rs` into `provision.rs`, `reprovision.rs`, and `shared.rs`

This ExecPlan is a living document. The sections **Progress**, **Surprises & Discoveries**, **Decision Log**, and **Outcomes & Retrospective** must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (this repo, branch `refactor/provision-outcome`, base `feat/reprovision`, PR #51, `mode:push`) | read-write | Move existing items between sibling modules under `agent/src/provision/`. No semantic changes; no new files outside `agent/src/provision/`. |

This plan lives in `plans/backlog/` of the agent repo. The orchestrator promotes it to `plans/active/` when work begins.

## Purpose / Big Picture

`agent/src/provision/entry.rs` has grown to 326 lines and now mixes the `provision` flow, the `reprovision` flow, and shared helpers (`cleanup_temp_dir`, `read_token_from_env`, `build_settings`) plus three independent unit-test submodules. Splitting it into `provision.rs`, `reprovision.rs`, and `shared.rs` puts each flow next to its own tests and isolates the shared helpers behind `pub(super)` visibility. The split is purely mechanical — no signatures, semantics, or test bodies change. This unlocks future work on either flow without bloating a single 300+-line file.

## Progress

- [x] (2026-04-29) M1: Create `agent/src/provision/shared.rs` with `cleanup_temp_dir`, `build_settings`, `read_token_from_env`, `TOKEN_ENV_VAR`, and the `read_token_from_env` test submodule.
- [x] (2026-04-29) M2: Create `agent/src/provision/provision.rs` with `ProvisionOutcome`, `provision`, `provision_with_backend`, `determine_settings`, and the `determine_settings` test submodule.
- [x] (2026-04-29) M3: Create `agent/src/provision/reprovision.rs` with `reprovision`, `reprovision_with_backend`, `determine_reprovision_settings`, and the `determine_reprovision_settings` test submodule.
- [x] (2026-04-29) M4: Update `agent/src/provision/mod.rs` to declare the new modules and re-export. Delete `agent/src/provision/entry.rs`.
- [ ] (YYYY-MM-DD) M5: Validation — `cargo build -p miru-agent --features test`, `cargo build -p miru-agent`, then `./scripts/preflight.sh` reports `Preflight clean`. (Implementation-stage validation passed: both builds clean, full `cargo test --features test` shows 1267 passed; preflight handoff to next stage.)

Use timestamps when you complete steps. Split partially completed work into "done" and "remaining" as needed.

## Surprises & Discoveries

- (2026-04-29) The working tree on `refactor/provision-outcome` had uncommitted modifications to `agent/src/main.rs` and `agent/src/provision/entry.rs` (changing `device: backend_client::Device` back to `Option<backend_client::Device>` and adapting the call site) when the implementation stage started. These changes contradicted the committed shape on this branch (HEAD had `pub device: backend_client::Device` plus an idempotency short-circuit that fabricates a `Device` from `crate::models::Device`) and broke the integration tests in `agent/tests/provision/entry.rs`, which still asserted `device.id`/`device.name` directly. Resolution: discarded the working-tree modifications to `main.rs`, used the HEAD-committed `entry.rs` as the verbatim source for the split, and left `main.rs` untouched per the orchestrator instructions.
- (2026-04-29) The HEAD `entry.rs` `provision()` body imports `crate::models` for the `read_json::<models::Device>()` call. That import had to come along with `provision()` into `provision.rs`; it was previously absorbed into the entry-file-wide imports.

## Decision Log

- (2026-04-29) Trusted the source file (HEAD-committed `entry.rs`) over the plan prose for the `ProvisionOutcome.device` field shape, per orchestrator guidance. Final file uses `pub device: backend_client::Device` with the doc comment, matching what `agent/tests/provision/entry.rs` and `agent/src/main.rs` already expect.
- (2026-04-29) Added `use crate::models;` to `provision.rs` (only) — `reprovision.rs` doesn't need it because reprovision has no idempotency short-circuit.
- (2026-04-29) Did not need `#[allow(clippy::module_inception)]` on `pub mod provision;` — `cargo build` was clean without it. Preflight may surface this lint and require adding it; deferred to that stage.

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

### Key files

- `agent/src/provision/entry.rs` — current 326-line source containing all four logical groups; deleted at end of M4.
- `agent/src/provision/mod.rs` — currently `pub mod display; pub mod entry; pub mod errors; pub use self::entry::*; pub use self::errors::ProvisionErr;`. Updated in M4.
- `agent/src/provision/display.rs`, `agent/src/provision/errors.rs` — unchanged.
- `agent/src/main.rs`, `agent/tests/provision/entry.rs` — out of scope; the wildcard re-export in `mod.rs` keeps `provision::ProvisionOutcome`, `provision::provision`, `provision::reprovision`, `provision::read_token_from_env`, `provision::determine_settings`, `provision::determine_reprovision_settings` reachable without edits.

### Repo conventions

- **Imports**: every source file uses three groups separated by blank lines and labelled with `// standard crates`, `// internal crates`, `// external crates`. The lint runner enforces this.
- **Errors**: every error type derives `thiserror::Error` and implements `crate::errors::Error`. **No new error variants in this plan.**
- **Test gating**: unit tests live inline in `#[cfg(test)] mod tests { ... }`; the existing structure with three nested submodules (`read_token_from_env`, `determine_settings`, `determine_reprovision_settings`) carries over verbatim — each submodule moves to the file that owns the function under test.
- **Workflow**: `./scripts/preflight.sh` runs lint + tests + tools-lint + tools-tests in parallel and prints `Preflight clean` on success.

### Visibility constraints

- `cleanup_temp_dir` (currently private) -> `pub(super)` so `provision.rs` and `reprovision.rs` can call it as `super::shared::cleanup_temp_dir`.
- `build_settings` (currently private) -> `pub(super)` so `provision.rs` and `reprovision.rs` can call it as `super::shared::build_settings`.
- `TOKEN_ENV_VAR` (currently private) stays private to `shared.rs`; verified by `grep -rn "TOKEN_ENV_VAR" agent/src/` showing the only usages are inside `read_token_from_env` itself.
- `ProvisionOutcome`, `provision`, `reprovision`, `read_token_from_env`, `determine_settings`, `determine_reprovision_settings` keep their existing `pub` visibility.
- `provision_with_backend`, `reprovision_with_backend` keep their existing private visibility (only called from the same file).

### Glossary

- **`shared.rs`**: holds helpers used by both `provision.rs` and `reprovision.rs`. Module is `mod shared;` (not `pub mod`) in `mod.rs`; the one externally-needed function (`read_token_from_env`) is re-exported by name from `mod.rs`. The other two helpers are crate-internal at `pub(super)` and intentionally not re-exported.

## M1 — Create `agent/src/provision/shared.rs`

**Goal**: extract the shared helpers from `entry.rs` into a new sibling file. Move the `read_token_from_env` test submodule with them.

**Files touched**:
- `agent/src/provision/shared.rs` (new)

**Contents** (move verbatim from current `entry.rs`):

- Imports needed: `std::env`, `crate::filesys`, `crate::provision::errors::*`, `crate::storage::settings`, and the `tracing` macros used.
- `const TOKEN_ENV_VAR: &str = "MIRU_PROVISIONING_TOKEN";` — keep private.
- `pub fn read_token_from_env() -> Result<String, ProvisionErr>` — verbatim.
- `pub(super) async fn cleanup_temp_dir(temp_dir: &filesys::Dir)` — bumped from private to `pub(super)`.
- `pub(super) fn build_settings(backend_host: Option<&str>, mqtt_broker_host: Option<&str>) -> settings::Settings` — bumped from private to `pub(super)`.
- `#[cfg(test)] mod tests` — the outer test scaffolding (the `lock_env()` fn and the `LOCK: OnceLock<Mutex<()>>` static) plus the inner `mod read_token_from_env` block. Drop the other two inner test modules — they move with their respective functions.

**Validation step**: file compiles after M2 and M3 also exist.

## M2 — Create `agent/src/provision/provision.rs`

**Goal**: house `ProvisionOutcome`, `provision`, `provision_with_backend`, and `determine_settings` in their own file, with the `determine_settings` test submodule alongside.

**Files touched**:
- `agent/src/provision/provision.rs` (new)

**Contents** (verbatim from current `entry.rs` — do not rewrite the struct shape; copy what's actually in the file):

- Imports (three groups, mirror `entry.rs` style):
  - internal: `crate::cli`, `crate::crypt::rsa`, `crate::filesys::Overwrite`, `crate::http`, `crate::provision::errors::*`, `crate::storage`, `crate::storage::settings`, `crate::version`, `backend_api::models as backend_client`, plus `super::shared::{build_settings, cleanup_temp_dir}`.
  - external: `#[allow(unused_imports)] use tracing::{debug, error, info, warn};` (preserve as-is from current file).
- `pub struct ProvisionOutcome` and its doc comment — copy verbatim from the source.
- `pub async fn provision<HTTPClientT: http::ClientI>(...) -> Result<ProvisionOutcome, ProvisionErr>` — body verbatim. The internal call `cleanup_temp_dir(&temp_dir).await` resolves via the `use super::shared::cleanup_temp_dir;` import.
- `async fn provision_with_backend<HTTPClientT: http::ClientI>(...) -> Result<backend_client::Device, ProvisionErr>` — body verbatim.
- `pub fn determine_settings(args: &cli::ProvisionArgs) -> settings::Settings` — body verbatim. Calls `build_settings(...)` via the `use super::shared::build_settings;` import.
- `#[cfg(test)] mod tests` — only contains the inner `mod determine_settings { ... }` block.

**Validation step**: `cargo build -p miru-agent --features test` succeeds after M3 and M4 also land.

## M3 — Create `agent/src/provision/reprovision.rs`

**Goal**: house `reprovision`, `reprovision_with_backend`, and `determine_reprovision_settings` with the `determine_reprovision_settings` test submodule alongside.

**Files touched**:
- `agent/src/provision/reprovision.rs` (new)

**Contents**:

- Imports (three groups):
  - internal: `crate::cli`, `crate::crypt::rsa`, `crate::filesys::Overwrite`, `crate::http`, `crate::provision::errors::*`, `crate::storage`, `crate::storage::settings`, `crate::version`, `backend_api::models as backend_client`, plus `super::shared::{build_settings, cleanup_temp_dir}`.
  - external: `#[allow(unused_imports)] use tracing::{debug, error, info, warn};`.
- `pub async fn reprovision<HTTPClientT: http::ClientI>(...) -> Result<backend_client::Device, ProvisionErr>` — body verbatim. Uses `cleanup_temp_dir` via the `super::shared` import.
- `async fn reprovision_with_backend<HTTPClientT: http::ClientI>(...) -> Result<backend_client::Device, ProvisionErr>` — body verbatim.
- `pub fn determine_reprovision_settings(args: &cli::ReprovisionArgs) -> settings::Settings` — body verbatim.
- `#[cfg(test)] mod tests` — only the inner `mod determine_reprovision_settings { ... }`.

**Validation step**: `cargo build -p miru-agent --features test` succeeds after M4 also lands.

## M4 — Update `mod.rs` and delete `entry.rs`

**Goal**: wire up the new modules and remove the old file.

**Files touched**:
- `agent/src/provision/mod.rs`
- `agent/src/provision/entry.rs` (deleted)

**`mod.rs` final contents**:

```rust
pub mod display;
pub mod errors;

pub mod provision;
pub mod reprovision;
mod shared;

pub use self::errors::ProvisionErr;
pub use self::provision::*;
pub use self::reprovision::*;
pub use self::shared::read_token_from_env;
```

The wildcard re-exports of `provision::*` and `reprovision::*` preserve the existing call-site paths (`provision::provision`, `provision::ProvisionOutcome`, `provision::reprovision`, `provision::determine_settings`, `provision::determine_reprovision_settings`) so `agent/src/main.rs` and `agent/tests/provision/entry.rs` need no edits. Note: the *child* module is named `provision` and the *parent* is also named `provision`; Rust handles this without ambiguity, but if a clippy `module_inception` lint fires, add `#[allow(clippy::module_inception)] pub mod provision;` rather than renaming.

`shared` is `mod` (not `pub mod`) because only `read_token_from_env` is publicly needed; `cleanup_temp_dir` and `build_settings` remain crate-internal at `pub(super)`.

**Delete `entry.rs`** with `git rm agent/src/provision/entry.rs` (or equivalent). Ensure no stale `pub mod entry;` line remains in `mod.rs`.

**Validation step**: `cargo build -p miru-agent --features test` and `cargo build -p miru-agent` (no `test` feature) both clean. `grep -rn "provision::entry" agent/` returns nothing.

## M5 — Validation

Run from the repo root:

1. `cargo build -p miru-agent --features test` — clean.
2. `cargo build -p miru-agent` — clean.
3. `cargo test -p miru-agent --features test provision::` — every `provision_fn::*` and `reprovision_fn::*` integration case plus the three relocated unit-test submodules pass.
4. `./scripts/preflight.sh` — final line reads `Preflight clean`.

> **Validation gate**: `./scripts/preflight.sh` MUST report `Preflight clean` before the push lands.

If preflight fails:

- **Lint** (`module_inception`, unused-imports, fmt): the new files preserve the `#[allow(unused_imports)] use tracing::{...};` line from the original; if clippy flags individually unused tracing macros per file, the `#[allow]` already covers them. If `clippy::module_inception` fires on `pub mod provision;`, add `#[allow(clippy::module_inception)]` immediately above that line in `mod.rs`.
- **Visibility errors**: most likely "private function `cleanup_temp_dir`" — confirm both helpers are `pub(super)` in `shared.rs`.
- **Path errors**: most likely an `unresolved import super::shared::...` in `provision.rs` or `reprovision.rs` — verify `mod shared;` is declared in `mod.rs` (without `pub`).
- **Test failures**: the three test submodules are moved verbatim with their `use super::*;` lines intact; failures here mean the test scaffolding (`lock_env`, the `OnceLock<Mutex<()>>`) was not carried over to `shared.rs` along with the `read_token_from_env` submodule. Re-check M1.

Re-run `./scripts/preflight.sh` until the final line is `Preflight clean`.

## Validation and Acceptance

The change is accepted when ALL of the following hold:

1. `./scripts/preflight.sh` exits 0 and prints `Preflight clean`. **This gate is non-negotiable.**
2. `agent/src/provision/entry.rs` no longer exists.
3. `agent/src/provision/` contains exactly: `display.rs`, `errors.rs`, `mod.rs`, `provision.rs`, `reprovision.rs`, `shared.rs`.
4. `grep -n "pub use" agent/src/provision/mod.rs` shows the three re-exports (`ProvisionErr`, `provision::*`, `reprovision::*`) plus the named `read_token_from_env` re-export.
5. `grep -rn "provision::entry" agent/` returns no matches.
6. `cargo test -p miru-agent --features test provision::` passes every previously-passing test with no test-body changes.
7. No edits land in `agent/src/main.rs`, `agent/tests/provision/entry.rs`, `agent/src/provision/display.rs`, or `agent/src/provision/errors.rs`.

## Idempotence and Recovery

- All edits are pure source moves; rerunning a milestone re-creates identical content.
- If a milestone aborts mid-way, `git status` shows the partial state and `git restore -SW agent/src/provision/` reverts cleanly.
- `mode:push`: this branch (`refactor/provision-outcome`) is already pushed to PR #51 with base `feat/reprovision`. After preflight is clean, commit the split as a single refactor commit and push to update the PR. No new branch, no new PR.
