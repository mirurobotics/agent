# Standardize the DeletePolicy enum onto the shared status-enum conventions

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (this repo, branch `feat/uploads-read-path`) | read-write | Extend the `impl_status_enum!` macro, route `DeletePolicy` through it, and migrate its tests to the shared status-enum harness. |
| `agent/libs/backend-api/` | read-only | Confirms the generated `backend_client::UploadDeletePolicy` enum variants the macro maps from. |

This plan lives in `agent/plans/backlog/` because every edit is to source and test files inside the `agent` repository.

Note on paths: the repository root is the working directory `repos/agent`, and the Rust crate lives under the `agent/` subdirectory of that root (e.g. `agent/src/models/status.rs`). All commands below run from the repository root (`repos/agent`) unless stated otherwise.

## Purpose / Big Picture

`DeletePolicy` (in `agent/src/models/upload_rule.rs`) is a backend-only status enum whose forward-compatible behavior — unknown wire strings and unknown backend values fall back to a default with a log line instead of failing — is currently hand-rolled in three separate `impl` blocks, with a parallel hand-written test module. Every other status enum in the repo (`DeviceStatus`, `DplTarget`, `DplActivity`, `DplErrStatus`, `DplStatus`) gets this behavior from the shared `impl_status_enum!` macro plus the `status_serde_tests!` harness.

After this change, `DeletePolicy` is generated the same way as the other enums. The observable outcome: `scripts/preflight.sh` reports `Preflight clean` (lint, clippy with `-D warnings`, fmt, cargo-diet deadcode, the full test suite, and `covgate.sh` with the `models` gate at 100%), and the new macro-generated `delete_policy_mapping_tests` plus the `status_serde_tests!(DeletePolicy)` suite pass — proving the same deserialize/serialize/forward-compat contract the manual code provided, with less hand-written code. Existing enums remain byte-for-byte behaviorally unchanged, proving the macro extension is purely additive.

## Progress

- [x] (2026-06-26) Milestone 1 — add the backend-only form to `impl_status_enum!` and refactor the shared `@core` arm. Committed; device + deployment enum tests (94 lib + integration) pass unchanged, proving additivity.
- [x] (2026-06-26) Milestone 2 — route `DeletePolicy` through the new macro form in `upload_rule.rs`. Committed; generated `delete_policy_mapping_tests` (3 tests) pass.
- [x] (2026-06-26) Milestone 3 — migrate `DeletePolicy` tests to `StatusFixture` + `status_serde_tests!`. Committed; 8 hand-written tests removed, replaced by 4 harness tests + `delete_policy_default`.
- [~] Milestone 4 — validation: build + `clippy --all-targets -D warnings` clean; all 100 model integration tests pass (device/deployment unchanged + new DeletePolicy). Full `scripts/preflight.sh` deferred to a later step per task scope.

## Surprises & Discoveries

- The integration-test binary target is `mod` (entry `tests/mod.rs`), not `models`; filter model tests via `--test mod 'models::'`.
- Pre-existing unrelated test failure: `logs_init_locked::test_reload_level_no_op_when_env_filter_locked` fails on the clean tree too (verified via `git stash`); it is environment-specific (global tracing env-filter lock) and independent of this change.

## Decision Log

- Decision: Add a new public backend-only macro form (no `agent_type`) rather than synthesizing a fake agent type for `DeletePolicy`.
  Rationale: `DeletePolicy` has no device/agent-API counterpart; it only converts from the backend client type. Forcing an `agent_type` would require inventing an unused conversion.
  Date/Author: 2026-06-26 / plan author.

## Context and Orientation

A reader needs no prior repo knowledge. Key facts:

### The macro: `agent/src/models/status.rs`

`impl_status_enum!` is a `macro_rules!` macro (re-exported as `pub(crate) use impl_status_enum;`). It currently has these arms, in order:

1. Agent base public form (lines 4–31): keyword `agent_type:`, 3-part mappings `$variant => $wire => $agent_value`. Delegates to the internal `@base` arm.
2. Backend-with-tests public form (lines 32–94): keywords `agent_type:` + `backend_type:` + `unknown_backend:`, 4-part mappings `$variant => $wire => $agent_value => $backend_value`. Delegates to arm 3, then emits a `#[cfg(test)] paste::paste!{ mod [<$name:snake _mapping_tests>] {...} }` with three tests: `unknown_backend_maps_to_default`, `unknown_wire_string_deserializes_to_default`, `known_backend_values_map_exactly`.
3. Plain backend public form (lines 95–152): keywords `agent_type:` + `backend_type:` (no `unknown_backend:`), 4-part mappings. Delegates to `@base`, then emits `From<&$name> for $backend_type` (domain→backend) and `From<&$backend_type> for $name` (backend→domain, unknown→default+`$log_macro!`).
4. Internal `@base` arm (lines 153–210): generates the `Deserialize` impl (unknown wire string → `$default` + `$log_macro!`), `variants()`, `as_str()`, and `From<&$name> for $agent_type`.

Macro arms are matched by exact token sequence; the literal keywords `agent_type:`, `backend_type:`, `unknown_backend:` disambiguate the forms.

### Callers that MUST stay byte-for-byte behaviorally unchanged

- `agent/src/models/device.rs:20` — `DeviceStatus` uses the agent base form (arm 1).
- `agent/src/models/deployment.rs:24,58,98,154` — `DplTarget`, `DplActivity`, `DplErrStatus`, `DplStatus` use the backend-with-tests form (arm 2).

### DeletePolicy today: `agent/src/models/upload_rule.rs:12–68`

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub enum DeletePolicy { #[default] Never, AfterUpload }

with three hand-written impls: `Display` (lines 21–28), `From<&backend_client::UploadDeletePolicy>` (30–47, unknown→`Never`+`error!`), and a custom `Deserialize` (49–68, unknown wire string→`Never`+`error!`). `tracing::error` is already imported at line 9. `backend_api::models as backend_client` is imported at line 3.

Verified: `DeletePolicy`'s `Display`/`to_string` is unused in production. The only `src/` references are its definition in `upload_rule.rs` and the re-export `pub use self::upload_rule::DeletePolicy;` in `agent/src/models/mod.rs:26`. Dropping `Display` is safe.

### Backend enum: `agent/libs/backend-api/src/models/upload_delete_policy.rs`

    pub enum UploadDeletePolicy {
        UPLOAD_DELETE_POLICY_NEVER,         // known
        UPLOAD_DELETE_POLICY_AFTER_UPLOAD,  // known
        UploadDeletePolicyUnknown,          // the unknown/catch-all variant
    }

VERIFIED unknown-variant path: `backend_client::UploadDeletePolicy::UploadDeletePolicyUnknown`. VERIFIED known-variant paths: `backend_client::UploadDeletePolicy::UPLOAD_DELETE_POLICY_NEVER` and `...::UPLOAD_DELETE_POLICY_AFTER_UPLOAD`.

### Test harness: `agent/tests/models/harnesses.rs`

`StatusFixture` (lines 231–240) requires `Serialize + DeserializeOwned + PartialEq + Eq + Hash + Debug + Default` and defines `variants()`, `cases()`, `wire_str()`. `status_serde_tests!($type)` (lines 318–345) emits a `mod harness` with four tests: `serde_roundtrip_all_variants`, `unknown_falls_back_to_default`, `rejects_invalid_string`, `as_str_matches_serde`. Reference implementation pattern: `agent/tests/models/device.rs:92–123` (`impl StatusFixture for DeviceStatus` + `mod status { status_serde_tests!(DeviceStatus); }` + a separate `status_default` test).

### DeletePolicy tests today: `agent/tests/models/upload_rule.rs:157–221`

`pub mod delete_policy` with eight hand-written tests: `default_is_never`, `display`, `from_backend_known`, `from_backend_unknown_defaults_to_never`, `deserialize_known`, `deserialize_unknown_defaults_to_never`, `deserialize_non_string_errors`, `serialize_is_snake_case`. Imports at line 10 are `use crate::models::harnesses::{serde_tests, ModelFixture, OptionalField, RequiredField};`.

### Coverage gate

`agent/src/models/.covgate` contains `100`, so the `models` module must keep 100% line coverage. The macro-generated `From<&$backend_type>` impl's unknown arm and known arm, and the `Deserialize` unknown arm, are the branches at risk; they are covered by the generated `delete_policy_mapping_tests` (in src, under `#[cfg(test)]`) and `status_serde_tests!` (in the integration tests). Confirm during validation.

## Plan of Work

### Milestone 1 — extend the macro (`agent/src/models/status.rs`)

Make the change purely additive and de-duplicate the shared core:

1. Add a new internal `@core` arm that generates only the truly shared behavior: the `Deserialize` impl (unknown wire string → `$default` + `$log_macro!`), and the `impl $name { pub fn variants(); pub fn as_str(); }` block. It takes `enum $name`, `default:`, `label:`, `log:`, and 2-part `mappings: [ $variant => $wire ],+`. It does NOT take `agent_type` and does NOT emit any `From`.

2. Refactor the existing `@base` arm so it delegates to `@core` for the shared core and then adds only `impl From<&$name> for $agent_type` (the agent conversion). This must leave the agent base form and both backend forms behaviorally identical — they already route through `@base`, so the only change is that `@base` now calls `@core` instead of inlining the `Deserialize`/`variants`/`as_str` code.

3. Add the new public backend-only form. Signature: `enum $name`, `default:`, `label:`, `log:`, `backend_type: $backend_type:ty`, `unknown_backend: $unknown_backend:path`, `mappings: [ $variant:ident => $wire:literal => $backend_value:path ],+`. Note: it has `backend_type:` + `unknown_backend:` but NO `agent_type:`, and 3-part mappings — a token sequence distinct from all existing arms, so it cannot shadow or be shadowed. It generates:
   - the shared core via `@core` (passing the 2-part `$variant => $wire` mappings);
   - `impl From<&$backend_type> for $name` matching each `$backend_value => $name::$variant`, with an `other =>` arm that logs via `$log_macro!` (mirror the wording in arm 3's backend→domain impl and the manual `From` in `upload_rule.rs`) and returns `$name::$default`;
   - the same `#[cfg(test)] paste::paste!{ mod [<$name:snake _mapping_tests>] {...} }` block arm 2 emits (the three tests `unknown_backend_maps_to_default`, `unknown_wire_string_deserializes_to_default`, `known_backend_values_map_exactly`).
   - It must NOT generate a domain→backend `From<&$name> for $backend_type`.

4. Update the top-of-file doc comment (lines 1–3) to mention the new backend-only form.

### Milestone 2 — route DeletePolicy through the new form (`agent/src/models/upload_rule.rs`)

1. Add `Hash` to the derive and keep the rest: `#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize)]`, keep `#[serde(rename_all = "snake_case")]` and `#[default] Never`.
2. Delete the manual `impl std::fmt::Display for DeletePolicy` (lines 21–28), the manual `impl From<&backend_client::UploadDeletePolicy> for DeletePolicy` (30–47), and the manual `impl<'de> Deserialize<'de> for DeletePolicy` (49–68).
3. Add `use crate::models::status::impl_status_enum;` to the internal-crates import group (mirror `device.rs:3` / `deployment.rs:3`). Keep `use tracing::error;` (already present, now consumed by the macro as the `log` argument).
4. Invoke the new form:

        impl_status_enum!(
            enum DeletePolicy,
            default: Never,
            label: "upload delete policy",
            log: error,
            backend_type: backend_client::UploadDeletePolicy,
            unknown_backend: backend_client::UploadDeletePolicy::UploadDeletePolicyUnknown,
            mappings: [
                Never => "never" =>
                    backend_client::UploadDeletePolicy::UPLOAD_DELETE_POLICY_NEVER,
                AfterUpload => "after_upload" =>
                    backend_client::UploadDeletePolicy::UPLOAD_DELETE_POLICY_AFTER_UPLOAD,
            ]
        );

### Milestone 3 — migrate tests (`agent/tests/models/upload_rule.rs`)

1. Extend the harness import (line 10) to add `StatusCase, StatusFixture, status_serde_tests` so it reads `use crate::models::harnesses::{serde_tests, status_serde_tests, ModelFixture, OptionalField, RequiredField, StatusCase, StatusFixture};` (mirror `device.rs:14–17`).
2. Replace the whole `pub mod delete_policy { ... }` block (lines 157–221) with, mirroring `device.rs:92–129`:
   - a top-level `impl StatusFixture for DeletePolicy` whose `variants()` returns `DeletePolicy::variants()`, `wire_str()` returns `self.as_str()`, and `cases()` returns the two known cases (`"\"never\""` → `Never` valid, `"\"after_upload\""` → `AfterUpload` valid) plus one unknown case (`"\"unknown\""` → `DeletePolicy::Never`, `valid: false`);
   - `mod delete_policy { use super::*; status_serde_tests!(DeletePolicy); }`;
   - a single explicit default test mirroring `device.rs`'s `status_default` (e.g. `fn delete_policy_default()` asserting `DeletePolicy::default() == DeletePolicy::Never`).
3. Remove all eight previous hand-written tests. The generated `delete_policy_mapping_tests` covers backend known/unknown conversion; `status_serde_tests!` covers serialize/deserialize roundtrip, unknown→default, invalid-string rejection, and `as_str`. Do not add tests device.rs does not have.

## Concrete Steps

All commands run from the repository root `repos/agent`.

1. Milestone 1: edit `agent/src/models/status.rs` per Plan of Work. Then confirm the existing enums still compile and their generated tests still pass (proves additivity):

        ./scripts/test.sh -- models::device models::deployment

   Expect the deployment/device model tests and their `*_mapping_tests` to pass. Commit:

        cd agent && git add src/models/status.rs && git commit -m "feat(models): add backend-only form to impl_status_enum macro" && cd ..

   (If on `main`, branch first; this work targets `feat/uploads-read-path`.)

2. Milestone 2: edit `agent/src/models/upload_rule.rs` per Plan of Work. Build:

        ./scripts/test.sh -- models::upload_rule

   Expect the upload_rule model tests plus the generated `delete_policy_mapping_tests` to pass. Commit:

        cd agent && git add src/models/upload_rule.rs && git commit -m "refactor(models): route DeletePolicy through impl_status_enum macro" && cd ..

3. Milestone 3: edit `agent/tests/models/upload_rule.rs` per Plan of Work. Run:

        ./scripts/test.sh -- models::upload_rule

   Expect the `delete_policy::harness::*` tests (`serde_roundtrip_all_variants`, `unknown_falls_back_to_default`, `rejects_invalid_string`, `as_str_matches_serde`), `delete_policy_default`, and the existing `from_backend*`/`defaults` tests to pass. Commit:

        cd agent && git add tests/models/upload_rule.rs && git commit -m "test(models): migrate DeletePolicy to status_serde_tests harness" && cd ..

4. Milestone 4: full validation (see next section).

## Validation and Acceptance

From the repository root `repos/agent`:

1. Run the full preflight gate:

        ./scripts/preflight.sh

   It runs `scripts/lint.sh` (custom import linter + field-by-field-assert check, `cargo fmt --check`, machete/cargo-diet deadcode, security audit, `cargo clippy ... -- -D warnings`) and `scripts/covgate.sh` (full `cargo test --features test` suite + per-module coverage thresholds, including `agent/src/models/.covgate` = 100). Acceptance: the final line printed is exactly

        Preflight clean

2. Confirm the macro extension is purely additive: the device and deployment status-enum tests must pass unchanged. They are exercised by step 1; you can also run them in isolation:

        ./scripts/test.sh -- models::device models::deployment

   Expect all to pass with no edits to `device.rs`/`deployment.rs`.

3. Coverage confirmation: `covgate.sh` must keep `models` at 100%. The branches introduced by the macro for `DeletePolicy` are: the backend→domain known arm and unknown (`other =>`) arm in the generated `From<&UploadDeletePolicy>`, and the `Deserialize` unknown arm. These are covered respectively by the generated `delete_policy_mapping_tests::known_backend_values_map_exactly` / `unknown_backend_maps_to_default` and by `status_serde_tests!`'s `unknown_falls_back_to_default`. If `covgate.sh` reports `models` below 100, inspect its line-level report for the uncovered arm and confirm the corresponding generated test exists and runs (note: the `mapping_tests` module lives in `src/` under `#[cfg(test)]`).

Acceptance summary: `Preflight clean` printed; `DeletePolicy` has no hand-written `Display`/`Deserialize`/`From` impls remaining; `models` coverage = 100; device/deployment enum tests pass unchanged.

## Idempotence and Recovery

- All edits are deterministic source edits; re-running `scripts/test.sh` / `scripts/preflight.sh` is safe and repeatable.
- Each milestone is a separate commit, so any milestone can be reverted independently with `git revert <sha>` (run from `agent/`) without disturbing the others.
- If the new macro form fails to match (e.g. a macro arm shadowing issue), the compiler error points at the `impl_status_enum!` call in `upload_rule.rs`; recovery is to verify the new arm's token sequence (`backend_type:` + `unknown_backend:`, no `agent_type:`, 3-part mappings) is distinct from arms 1–3, then rebuild. No data or external state is touched, so there is no cleanup beyond `git`.

## Outcomes & Retrospective

Completed Milestones 1–3 across three commits on `feat/uploads-read-path`:

1. `feat(models): add backend-only form to impl_status_enum macro` — extracted shared `Deserialize`/`variants()`/`as_str()` into a new internal `@core` arm; `@base` now delegates to `@core` and adds only the domain→agent `From`; added the public backend-only form (`backend_type:` + `unknown_backend:`, no `agent_type:`, 3-part mappings) that emits `@core` + backend→domain `From` (unknown→default+log) + the `*_mapping_tests` module, and NO domain→backend `From`.
2. `refactor(models): route DeletePolicy through impl_status_enum macro` — added `Hash` to the derive, removed the three hand-written impls, invoked the new form.
3. `test(models): migrate DeletePolicy to status_serde_tests harness` — added `impl StatusFixture for DeletePolicy` + `status_serde_tests!(DeletePolicy)` + `delete_policy_default`, removed all eight hand-written tests.

Validation done within task scope: `cargo build --features test` clean, `cargo clippy --features test --all-targets -- -D warnings` clean, all 100 model integration tests pass (device + deployment status enums behaviorally unchanged; new DeletePolicy coverage in place). Full `scripts/preflight.sh` (incl. covgate 100% on models) intentionally deferred to a later step.

DeletePolicy is now covered by: generated `delete_policy_mapping_tests::{unknown_backend_maps_to_default, unknown_wire_string_deserializes_to_default, known_backend_values_map_exactly}` (src, `#[cfg(test)]`) and harness `delete_policy::harness::{serde_roundtrip_all_variants, unknown_falls_back_to_default, rejects_invalid_string, as_str_matches_serde}` plus `delete_policy_default`.
