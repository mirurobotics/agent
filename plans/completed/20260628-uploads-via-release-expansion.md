# Uploads: re-vendor openapi #151 and move upload-rule acquisition to the RELEASE expansion

This ExecPlan is a living document. The sections **Progress**, **Surprises & Discoveries**, **Decision Log**, and **Outcomes & Retrospective** MUST be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `/home/ben/miru/workbench4/repos/agent` (the Rust device agent — the only repo edited) | read-write | Re-vendor `api/specs/backend/v04.yaml` from openapi `#151` and regenerate `libs/backend-api` models; move upload-rule acquisition from the DEPLOYMENT-level expansion to the RELEASE-level expansion (request `release.upload_rules` instead of `upload_rules`; source rules from the expanded `Release` object, not the `Deployment`). |
| `/home/ben/miru/workbench4/repos/openapi` | read-only | Source of the vendored bundle. Read `apis/apps/backend-server/agent/openapi.gen.yaml` at `origin/main` commit `97809d8`. No edits. |

This plan lives in `plans/backlog/` per the agent repo's plan conventions (`plans/{active,archived,backlog,completed}/`). Commit all changes from inside the agent repo's own git context (see workbench `CLAUDE.md`), never from the workbench root. Work happens on the **existing** branch `feat/uploads-via-deployment-expansion` (PR #90, base `main`); this is **push mode** — stay on that branch, do not create a new one.

This plan refactors the just-completed `plans/completed/20260628-uploads-via-deployment-expansion.md` (openapi `#150`, which delivered upload rules on the `Deployment` object). openapi `#151` moved the upload-rule expansion onto the agent `Release` object. This plan re-vendors that contract and rewires the agent to match. **It deliberately ENDS at "the deployed release's upload rules arrive via the `release.upload_rules` expansion and are cached in local state."** Nothing consumes the rules beyond storage.

### Explicitly OUT OF SCOPE (do NOT implement here)

These are intentionally NOT in this plan and MUST NOT be implemented (same scope boundary as the #150 plan):

- File discovery / glob matching on `source.glob`; the per-rule poll loop (`poll_interval_secs`); stability / finalization detection (`stability_window_secs`).
- Streaming sha256 digest + size computation over candidate files.
- `POST /uploads` (presigned `PUT`) and `POST /uploads/{upload_id}/confirm`.
- The local uploads ledger / idempotency / retry state.
- `delete_policy` enforcement (deleting local source files).
- Any background upload worker or `app/run.rs` integration.

KEEP unchanged (already correct from prior milestones): `agent/src/models/upload_rule.rs` (domain model + `From<BaseUploadRule>`), `agent/src/storage/upload_rules.rs` (persisted store, spawned in `storage/mod.rs`), `agent/src/storage/layout.rs` (`upload_rules()` path). The generated POST/confirm models (`Upload`, `PresignedUpload`, `CreateUploadRequest`, etc.) remain generated-but-unused — expected and fine.

## Purpose / Big Picture

openapi `#151` ("refactor(agent): move upload-rules expansion from Deployment to Release") changes where the agent learns its upload rules. In `#150` (the current branch's state) rules rode in as an array on the **Deployment** object, requested via the `upload_rules` expansion. `#151` REMOVES `Deployment.upload_rules` (and the `DEPLOYMENT_EXPAND_UPLOAD_RULES` / `DEPLOYMENT_LIST_EXPAND_UPLOAD_RULES` enum variants) and instead delivers rules as an expandable array on the **Release** object, requested via the nested `release.upload_rules` path — exactly mirroring the existing `release.git_commit` expansion. The agent must therefore:

1. Re-vendor the contract to pick up the new shape.
2. Request `release.upload_rules` (replacing `upload_rules`) on the deployment fetch the syncer already performs.
3. Extract + cache the rules from the **expanded release**, not the deployment.

This is a contained acquisition-mechanism swap with no change to downstream upload behavior (all still out of scope).

**Observable outcome at completion:** after a sync, the deployed release's upload rules are present in the agent's local state at `/var/lib/miru/resources/upload_rules.json`, having ridden in on the deployment fetch as part of the expanded `Release` (no separate HTTP call). The vendored spec has `Release.upload_rules` and no `Deployment.upload_rules`. `scripts/preflight.sh` prints `Preflight clean`.

## Progress

- [x] **S0** (2026-06-28) Preflight the generator — `openapi-generator-cli` 7.12.0 available; `agent/tests/mocks/http_client.rs` has no `upload_rules` refs (no edit needed).
- [x] **S1** (2026-06-28) Re-vendored `api/specs/backend/v04.yaml` at openapi `97809d8` (v0.5.0-pre stamp continued); diff verified as exactly the #151 field move + git-commit stamp; ran `api/regen.sh`; committed spec + regenerated models (`797bf18`).
- [x] **S2** (2026-06-28) Rewired `agent/src/sync/deployments.rs` to release-level acquisition; folded extraction into `store_expanded_release`, deleted the standalone helper, reworded the error (`7d2ce02`).
- [x] **S3** (2026-06-28) Updated tests/fixtures for release-sourced rules; committed (`92281f7`).
- [~] **V** Deferred to the orchestrator's later delivery step (task scope explicitly excludes running `scripts/preflight.sh`). Verified green here: `cargo build -p miru-agent --features test`, `cargo clippy -p miru-agent --features test --all-targets -- -D warnings`, `cargo fmt --check`, and the sync/models/storage test suites (incl. both new branches).

Use timestamps when completing steps. Split partially-completed work into "done" / "remaining".

## Surprises & Discoveries

(Add entries as work proceeds. Seed findings from the verified context below.)

- (Seed) The current branch ALREADY implements deployment-level acquisition end-to-end (from the #150 plan): `agent/src/sync/deployments.rs` has a `store_expanded_upload_rules` helper reading `backend_dpl.upload_rules`; `agent/src/sync/errors.rs` has the `UploadRulesNotExpanded` variant; tests exist. This plan REWORKS that code in place, it does not add it from scratch. Re-verify the exact line numbers below before editing (they drift).
- (Seed) There is NO dedicated `ReleaseExpansion` schema in the bundle. Nested expansions (`release.git_commit`, `release.upload_rules`) are plain `&str` literals in the syncer's expansions list — picking up `release.upload_rules` is a one-string change, no generated enum involved.
- (Seed) `agent/tests/mocks/http_client.rs` does NOT reference `upload_rules` at all (the deployment/release expansion fixtures live in `agent/tests/sync/helpers.rs`). The mock needs NO edits for this change. Re-verify with the grep in S0.
- (2026-06-28, S1) The vendor diff is EXACTLY #151 — no extra changes. Order-independent line comparison vs. the prior spec shows only: `Deployment.upload_rules` removed; `Release.upload_rules` added (description reworded to `expand=release.upload_rules`); the two `DEPLOYMENT_*_UPLOAD_RULES` enum varnames + `upload_rules` enum values removed; `info.x-git-commit` bumped to `97809d8`. The large +/- blocks in the raw `git diff` are pure YAML reordering of the `Release`/`ConfigInstance` schemas (they shifted position relative to `Deployment`), not content changes.
- (2026-06-28, S1) DEVIATION from the plan's codegen prediction: the generator (7.12.0) DOES include `upload_rules: None` inside `Release::new()` (line 50 of `libs/backend-api/src/models/release.rs`), alongside `git_commit: None` — the plan predicted it would be omitted from `new()`. Harmless and consistent: both optional fields default to `None` in `new()` and neither is a `new()` parameter.
- (2026-06-28, S3) DEVIATION: the plan's enumerated test surface missed two `BackendRelease { ... }` literals in `agent/tests/models/release.rs` (`from_backend` line ~66, `from_backend_invalid_dates` line ~88) that have no `..Default::default()`. Adding `upload_rules` to the `Release` struct made them fail to compile; fixed by adding `upload_rules: None,` to each (mirroring `git_commit: None`). All other `backend_client::Release` literals across the test tree use `..Default::default()` and needed no change.

## Decision Log

(Add entries as work proceeds.)

- **Decision: Continue `v0.5.0-pre` pre-release stamping** (do NOT invent a release version). No openapi agent release tag is newer than `agent/v0.4.0`; the contract is still pre-release. Only the `x-git-commit` block moves to `97809d8`. Rationale: matches the established convention from the prior vendors (fe6e9ca/e0bc63e). Date/Author: 2026-06-28 / plan author.
- **Decision: FOLD upload-rule extraction into `store_expanded_release`** (remove the separate `store_expanded_upload_rules` helper). Rationale: in `#151` `upload_rules` is a sibling of `git_commit` ON the `Release` object, so all release-expansion extraction belongs in one place; this faithfully mirrors the existing `git_commit` handling, removes a now-redundant helper and its separate call site in `pull_deployments`, and naturally gates upload-rule extraction on release presence (the contract nests `upload_rules` under `release`). ALTERNATIVE considered: keep a separate helper reading `backend_dpl.release.upload_rules` — rejected as it duplicates the "is the release expanded?" guard that `store_expanded_release` already owns. Date/Author: 2026-06-28 / plan author.
- **Decision: `UploadRulesNotExpanded` now fires when an EXPANDED release lacks `upload_rules`** (`release.upload_rules` is `None`), not on every deployment. If the release itself is absent (`backend_dpl.release` is `None`), no upload-rule check runs — the existing release handling already returns early, and the syncer requests `release.upload_rules`, so a present release always carries the array. Keep the variant name `UploadRulesNotExpanded`; keep the `deployment_id` field (still available as `backend_dpl.id` inside `store_expanded_release` and still the most useful identifier for an operator). Update only the error message wording to say the release lacked upload rules. Rationale: minimal, faithful mirror; a rename adds churn without value. Date/Author: 2026-06-28 / plan author.

## Outcomes & Retrospective

Completed 2026-06-28 on branch `feat/uploads-via-deployment-expansion` (push mode). Upload-rule acquisition now reads `release.upload_rules` end-to-end:

- **S1** (`797bf18`): re-vendored `api/specs/backend/v04.yaml` at openapi `97809d8`, regenerated `libs/backend-api` models. `Release` gained `upload_rules`, `Deployment` lost it, the two `DEPLOYMENT_*_UPLOAD_RULES` expansion variants are gone. Vendor diff verified as exactly #151.
- **S2** (`7d2ce02`): syncer requests `["config_instances", "release.git_commit", "release.upload_rules"]`; upload-rule extraction folded into `store_expanded_release` (placed before the git_commit early-return); `store_expanded_upload_rules` helper and its call deleted; `UploadRulesNotExpanded` (name + `deployment_id` field kept) now fires when an expanded release lacks `upload_rules`, message reworded.
- **S3** (`92281f7`): fixtures source rules from the expanded release; key tests `stores_upload_rules_from_expanded_release` (happy path) and `upload_rules_not_expanded_error` (None-array branch) pass.

Validation (this task's scope, not full preflight): `cargo build -p miru-agent --features test`, `cargo clippy -p miru-agent --features test --all-targets -- -D warnings`, `cargo fmt --check`, and the sync/models/storage suites all green. Full `scripts/preflight.sh` is left to the orchestrator's delivery step.

Deviations: generator put `upload_rules: None` in `Release::new()` (plan predicted omission — harmless); two `BackendRelease` literals in `agent/tests/models/release.rs` needed the new field (not in the plan's enumerated test surface). See Surprises & Discoveries.

## Context and Orientation

The agent is a Rust workspace rooted at `repos/agent/Cargo.toml`: `agent/` (binary crate — all logic), `libs/backend-api/` and `libs/device-api/` (OpenAPI-generated models; do NOT hand-edit — regenerate via `api/regen.sh`). Repo conventions live in `repos/agent/AGENTS.md`: import ordering (std / internal / external groups), `thiserror` errors implementing the custom `crate::errors::Error` trait, `#[cfg(feature = "test")]` gating, `scripts/test.sh` runs `RUST_LOG=off cargo test --features test` (the `--features test` flag is REQUIRED; mocks/helpers are behind it), per-module `.covgate` thresholds.

### What the spec change is (#150 → #151)

Current vendored spec (`api/specs/backend/v04.yaml`, stamped at openapi `e0bc63e` = #150) has:
- `Deployment.upload_rules: array<BaseUploadRule>` (line ~1121).
- `DeploymentExpansion` enum value `upload_rules` / varname `DEPLOYMENT_EXPAND_UPLOAD_RULES` (line ~1177).
- `DeploymentListExpansion` value `upload_rules` / varname `DEPLOYMENT_LIST_EXPAND_UPLOAD_RULES` (line ~528).

New bundle (openapi `97809d8` = #151) instead has:
- `Release.upload_rules: array<BaseUploadRule>` — a sibling of `git_commit` inside the `Release` `allOf` (described "Expand using 'expand=release.upload_rules'").
- `DeploymentExpansion` enum: only `release`, `config_instances` (NO `upload_rules`).
- `DeploymentListExpansion` enum: only `total_count`, `release`, `config_instances` (NO `upload_rules`).
- `BaseUploadRule`, `UploadRuleSource`, `UploadRuleDestination`, `UploadDeletePolicy` — UNCHANGED (still reached, now via the release expansion).

Full source commit: `97809d89ad2ad29405859e5709b0a29cd72ecc01`, subject `refactor(agent): move upload-rules expansion from Deployment to Release (#151)`.

### Vendoring & codegen surface (S1)

- `api/specs/backend/v04.yaml` — the vendored agent bundle. Current `info:` block (lines 2-13): `version: v0.5.0-pre`, `license`, `x-release-version: v0.5.0-pre`, `x-git-commit:{sha,url,message}` pinned to `e0bc63e`.
- `api/Makefile` `gen-backend` runs `npx --yes @openapitools/openapi-generator-cli generate -i specs/backend/v04.yaml -g rust -t templates/rust -o codegen/backend --additional-properties=packageName=backend-api`.
- `api/regen.sh` runs `make gen`, wipes `libs/backend-api/src/models/*` + `libs/device-api/src/models/*`, and copies freshly generated models in. It regenerates `libs/backend-api/src/models/mod.rs` too.
- `api/openapitools.json` pins the generator CLI version (7.12.0 was available for the #150 vendor).

Generated-model effects of the `e0bc63e → 97809d8` diff (verify after regen):
- `libs/backend-api/src/models/deployment.rs` — `Deployment` LOSES the `upload_rules: Option<Vec<models::BaseUploadRule>>` field (currently lines 49-51). The struct retains `release: Option<Box<models::Release>>` and `config_instances`.
- `libs/backend-api/src/models/release.rs` — `Release` GAINS `pub upload_rules: Option<Vec<models::BaseUploadRule>>` (rename `upload_rules`, `skip_serializing_if = "Option::is_none"`), a sibling of the existing `git_commit: Option<Option<Box<models::GitCommit>>>` field. Not added to `Release::new()` (matches how `git_commit` is omitted from `new()`).
- `libs/backend-api/src/models/deployment_expansion.rs` — LOSES the `DEPLOYMENT_EXPAND_UPLOAD_RULES` variant.
- `libs/backend-api/src/models/deployment_list_expansion.rs` — LOSES the `DEPLOYMENT_LIST_EXPAND_UPLOAD_RULES` variant.
- `libs/backend-api/src/models/base_upload_rule.rs` (+ `upload_rule_source.rs`, `upload_rule_destination.rs`, `upload_delete_policy.rs`) — UNCHANGED.
- `libs/backend-api/src/models/api_version.rs` — still renders `v0.5.0-pre` (no `Miru-Version` header change). `device-api` untouched.

### Read-path code touched (S2)

`agent/src/sync/deployments.rs` (current state, from #150) — verify these before editing:
- `Storage<'a>` struct (lines 27-33) carries `pub upload_rules: &'a storage::UploadRules` (line 32) — **KEEP** (the destination store is unchanged).
- `fetch_active_deployments` expansions literal (line 124): `&["config_instances", "release.git_commit", "upload_rules"]` — change `"upload_rules"` → `"release.upload_rules"`.
- `pull_deployments` (lines 90-111) currently calls both `store_expanded_release` (line 98) and `store_expanded_upload_rules` (line 99) — remove the `store_expanded_upload_rules` call (folded into `store_expanded_release`).
- `store_expanded_release` (lines 240-267) — the function to extend: it caches the release, then early-returns when `git_commit` is `None`. Insert the upload-rule extraction BETWEEN the release write and the git_commit early-return.
- `store_expanded_upload_rules` (lines 269-296) — DELETE this helper and its doc comment.

`agent/src/sync/errors.rs` (current state):
- `UploadRulesNotExpandedErr` struct (lines 73-79) — KEEP; update the `#[error(...)]` message wording (line 74) from "deployment '{deployment_id}' did not have upload_rules expansion" to reference the release, e.g. `"deployment '{deployment_id}' release did not have upload_rules expansion (backend did not expand release.upload_rules)"`. KEEP the `deployment_id` field.
- The `SyncErr::UploadRulesNotExpanded` enum variant (line 108), the `From` impl (lines 153-157), and the `impl_error!` entry (line 172) — all KEEP unchanged.

`agent/src/sync/syncer.rs::sync_impl` (line 241) — `upload_rules: storage_ref.upload_rules.as_ref(),` in the `deployments::Storage { ... }` literal — KEEP (the storage field is unchanged).

### Test surface (S3) — exact sites

Grep `upload_rules` across `agent/tests` to find every site. The ones that change:

- `agent/tests/sync/helpers.rs`:
  - `make_deployment` (line 33) currently sets `upload_rules: Some(Vec::new())` on the `BackendDeployment` literal (line 40) — **REMOVE that line** (the field no longer exists on `Deployment`).
  - `make_backend_release` (lines 67-78) — **ADD** `upload_rules: Some(Vec::new())` to the `BackendRelease` literal so every expanded release carries the array (otherwise the new required-expansion check trips). This mirrors how `make_deployment` formerly defaulted the deployment field.
  - `make_backend_upload_rule` (lines 80-87) — KEEP (builds a `BaseUploadRule`).
  - `make_deployment_with_upload_rules` (lines 89-103) — currently attaches rules to `dpl.upload_rules`. **REWORK** to attach rules to the deployment's expanded release instead: build a release (via `make_backend_release`), set its `upload_rules` to the rule list, and attach it as `dpl.release`. Rename to `make_deployment_with_release_upload_rules` (recommended) or keep the name; whichever, it must produce a deployment whose `release.upload_rules` carries the rules. (A release is now required to carry rules, so the helper MUST set a release.)
  - `assert_upload_rule_stored` (lines 186-192) — KEEP unchanged.
- `agent/tests/sync/deployments.rs`:
  - `stores_upload_rules_from_expanded_deployment` (lines 194-211) — rename to `stores_upload_rules_from_expanded_release` and build the deployment via the reworked helper so rules ride on the expanded release. Assertions (`assert_upload_rule_stored`) unchanged. **Covers the happy path: expanded release → rules cached via `write_if_absent`.**
  - `upload_rules_not_expanded_error` (lines 480-501) — rework the unexpanded deployment: it must have an expanded release whose `upload_rules` is `None` (currently it sets `dpl.upload_rules = None` on a release-less deployment). Build via `make_deployment_with_release(...)` then set `dpl.release.as_mut().unwrap().upload_rules = None`. Assertion (`SyncErr::UploadRulesNotExpanded(_)` or a `SyncErrors` containing it) unchanged. **Covers the new `ok_or_else` error branch.**
  - The `Fixture` (lines 30-104) already spawns `upload_rule_stor` and passes `upload_rules: &self.upload_rule_stor` — KEEP unchanged.
- `agent/tests/models/deployment.rs`:
  - `from_backend` (line 857) and `from_backend_invalid_dates` (line 905) build `backend_client::Deployment { ... }` literals that include `upload_rules: Some(vec![])` — **REMOVE both lines** (field gone from `Deployment`). These literals have no `..Default::default()`, so the removed field is the only change needed; `release: None` keeps them off the upload-rule path.
- `agent/tests/sync/syncer.rs`:
  - Three `backend_api::models::Deployment { ... }` literals at lines ~336, ~950, ~1042 include `upload_rules: Some(vec![])` (lines 346, 956, 1048) — **REMOVE all three lines**. Each literal has `release: None` (via `..Default::default()`), so no upload-rule check fires and no release fixup is needed.
  - The real-syncer fixture (lines 81, 93) spawns and wires `upload_rules` into `storage::Storage` — KEEP unchanged (`storage::Storage` still owns the `UploadRules` store).
- `agent/tests/sync/errors.rs` (lines 105-110, `upload_rules_not_expanded_err_maps`) — exercises the `From<UploadRulesNotExpandedErr>` mapping; KEEP unchanged (the variant/From are unchanged).
- `agent/tests/models/upload_rule.rs`, `agent/tests/storage/upload_rules.rs`, `agent/tests/storage/layout.rs`, `agent/tests/storage/caches.rs` — KEEP unchanged (storage/model wiring untouched; verify they still pass).
- `agent/tests/storage/mod.rs` (line 16) `upload_rules: 1000` capacity — unrelated config; KEEP.

### Validation tooling

- `scripts/preflight.sh` runs, in parallel: `scripts/lint.sh` (import linter, `cargo fmt`, `cargo machete`/diet, security audit, clippy `-D warnings`) and `scripts/covgate.sh` (tests + per-module coverage gate). Prints `Preflight clean` on success.
- Relevant `.covgate` minimums: `agent/src/models/` = **100**, `agent/src/http/` = **93.9**, `agent/src/storage/` = **94.83**, `agent/src/sync/` = **93.63**.
- `scripts/test.sh` = `RUST_LOG=off cargo test --features test`.
- Run `scripts/update-deps.sh` before linting to refresh `Cargo.lock`.

## Plan of Work

### S0 — Preflight the generator (gate)

Confirm the generator runs and confirm the mock needs no edits, before changing anything:

    cd /home/ben/miru/workbench4/repos/agent/api
    npx --yes @openapitools/openapi-generator-cli version
    cd /home/ben/miru/workbench4/repos/agent
    grep -n upload_rules agent/tests/mocks/http_client.rs   # expect: no hits

If `openapi-generator-cli version` fails (no network / npx / Java unavailable), **STOP and report**. Do NOT hand-write or hand-edit generated models in `libs/backend-api/src/models/` — `regen.sh` regenerates them wholesale and any hand-edit is silently destroyed and diverges from the contract.

### S1 — Re-vendor spec + regenerate models (one atomic commit)

1. Extract the source bundle (read-only) and capture commit metadata:

       cd /home/ben/miru/workbench4/repos/openapi
       git show origin/main:apis/apps/backend-server/agent/openapi.gen.yaml > /tmp/.../scratchpad/bundle-97809d8.yaml
       git show -s --format='%H%n%s' 97809d8
       # full sha:  97809d89ad2ad29405859e5709b0a29cd72ecc01
       # subject:   refactor(agent): move upload-rules expansion from Deployment to Release (#151)

2. Produce the stamped `api/specs/backend/v04.yaml`. The ONLY differences from the raw `97809d8` bundle are the vendoring stamp (mirror the current file exactly):
   - `info.version: 0.0.0` → `version: v0.5.0-pre`
   - keep the existing `license:` block (already matches)
   - `info.x-release-version: v0.5.0-pre`
   - `info.x-git-commit:` → `sha: 97809d89ad2ad29405859e5709b0a29cd72ecc01`, `url: https://github.com/mirurobotics/openapi/commit/97809d89ad2ad29405859e5709b0a29cd72ecc01`, `message: 'refactor(agent): move upload-rules expansion from Deployment to Release (#151)'`
   - substitute BOTH `$API_VERSION$` placeholders (the `APIVersion` enum value and the `MiruVersion` parameter `example:`) → `v0.5.0-pre`
   - leave everything else (servers/security/paths/components) exactly as the source bundle has it.

   Easiest faithful approach: copy the current `v04.yaml` `info:` block (lines 2-13) onto the new bundle's body, update only the three `x-git-commit` keys, and do the two `$API_VERSION$` substitutions.

3. **Re-verify the diff before regen.** The structural delta vs. the current vendored spec must be exactly:
   - `Deployment` schema: `upload_rules` property REMOVED.
   - `DeploymentExpansion`: `upload_rules` value + `DEPLOYMENT_EXPAND_UPLOAD_RULES` varname REMOVED.
   - `DeploymentListExpansion`: `upload_rules` value + `DEPLOYMENT_LIST_EXPAND_UPLOAD_RULES` varname REMOVED.
   - `Release` schema: `upload_rules: array<BaseUploadRule>` property ADDED.
   - `info.x-git-commit` → `97809d8`.

       cd /home/ben/miru/workbench4/repos/agent
       grep -nE '^  version:|x-release-version:|sha:|\$API_VERSION\$' api/specs/backend/v04.yaml   # version/release = v0.5.0-pre; no $API_VERSION$ left
       grep -nE 'DEPLOYMENT_EXPAND_UPLOAD_RULES|DEPLOYMENT_LIST_EXPAND_UPLOAD_RULES' api/specs/backend/v04.yaml  # expect NONE
       grep -nE 'release.upload_rules|expand=release.upload_rules' api/specs/backend/v04.yaml      # expect the Release.upload_rules description present
       # Confirm Deployment no longer has an upload_rules property and Release does:
       # (inspect the Deployment and Release schema blocks directly)

   If anything beyond the five changes above differs, **flag it for review** before continuing.

4. Regenerate and build the generated crate:

       api/regen.sh
       cargo build -p backend-api 2>&1 | tail -20
       grep -n upload_rules libs/backend-api/src/models/release.rs            # Release.upload_rules field present
       grep -n upload_rules libs/backend-api/src/models/deployment.rs         # expect NONE (field gone)
       grep -n UPLOAD_RULES libs/backend-api/src/models/deployment_expansion.rs       # expect NONE
       grep -n UPLOAD_RULES libs/backend-api/src/models/deployment_list_expansion.rs  # expect NONE
       grep -n 'API_VERSION\|rename' libs/backend-api/src/models/api_version.rs        # renders v0.5.0-pre (unchanged)

   `cargo build -p backend-api` succeeds (generated crate only). The full workspace build (`cargo build --features test`) will FAIL here because `agent` source/tests still reference `Deployment.upload_rules` — that is expected and fixed by S2/S3. Do NOT commit a broken workspace as the final state of S1; commit only the spec + generated models (the agent source is unchanged at this point, so the workspace failure is purely the to-be-rewired agent code, which lands in S2/S3).

5. Commit S1 (spec + regenerated `libs/backend-api/src/models/`) so the generated churn is isolated:
   `feat(api): re-vendor uploads contract from openapi 97809d8 (#151) — upload rules via release expansion`.

### S2 — Rewire acquisition to the release expansion (one commit)

All edits in `agent/src/sync/`.

1. **Request the nested expansion** — `agent/src/sync/deployments.rs::fetch_active_deployments` (line 124):

       let expansions: &[&str] = &["config_instances", "release.git_commit", "release.upload_rules"];

   (Replace `"upload_rules"` with `"release.upload_rules"`; keep the other two.)

2. **Fold extraction into `store_expanded_release`** (lines 240-267). After the release is written and BEFORE the `git_commit` early-return, extract + require + cache the rules from the release. The function becomes (shape):

       async fn store_expanded_release(
           storage: &Storage<'_>,
           backend_dpl: &backend_client::Deployment,
       ) -> Result<(), SyncErr> {
           let Some(backend_release) = backend_dpl.release.as_deref() else {
               return Ok(());
           };

           let release: models::Release = backend_release.clone().into();
           let release_id = release.id.clone();
           storage
               .releases
               .write_if_absent(release_id, release, |_, _| false)
               .await?;

           // upload rules ride on the expanded release; the syncer requests
           // `expand=release.upload_rules`, so a present release must carry the
           // array — a missing array is a contract violation (hard error).
           let rules = backend_release.upload_rules.clone().ok_or_else(|| {
               SyncErr::UploadRulesNotExpanded(UploadRulesNotExpandedErr {
                   deployment_id: backend_dpl.id.clone(),
               })
           })?;
           for backend_rule in rules {
               let rule: models::UploadRule = backend_rule.into();
               let id = rule.id.clone();
               storage
                   .upload_rules
                   .write_if_absent(id, rule, |_, _| false)
                   .await?;
           }

           let Some(Some(backend_gc)) = &backend_release.git_commit else {
               return Ok(());
           };
           let gc: models::GitCommit = (*backend_gc.clone()).into();
           let gc_id = gc.id.clone();
           storage
               .git_commits
               .write_if_absent(gc_id, gc, |_, _| false)
               .await?;

           Ok(())
       }

   The upload-rule block MUST sit before the `git_commit` early-return, or a release without a git commit would skip the required upload-rule check. Update the function's doc comment to mention it now also caches the expanded upload rules.

3. **Remove the standalone helper and its call:**
   - Delete `store_expanded_upload_rules` (lines 269-296) and its doc comment.
   - In `pull_deployments`, remove the `store_expanded_upload_rules(storage, &backend_dpl).await?;` line (line 99). The `store_expanded_release(...)` call (line 98) now handles both.

4. **Update the error message** — `agent/src/sync/errors.rs`, the `UploadRulesNotExpandedErr` `#[error(...)]` (line 74), to reference the release rather than the deployment-level expansion, e.g.:

       #[error("deployment '{deployment_id}' release did not have upload_rules expansion (backend did not expand release.upload_rules)")]

   KEEP the struct field `deployment_id`, the `SyncErr::UploadRulesNotExpanded` variant, the `From` impl, and the `impl_error!` entry exactly as they are.

5. **Verify** no remaining reference to the removed `Deployment.upload_rules` field in source:

       grep -rn 'dpl.upload_rules\|deployment.*\.upload_rules\|backend_dpl.upload_rules\|store_expanded_upload_rules' agent/src
       # expect: NO hits

6. Commit S2: `refactor(sync): acquire upload rules via the release expansion`.

### S3 — Tests (one commit)

Edit the exact sites listed in "Test surface (S3)". Summary of edits:

1. `agent/tests/sync/helpers.rs`: remove `upload_rules: Some(Vec::new())` from `make_deployment`; add `upload_rules: Some(Vec::new())` to `make_backend_release`; rework `make_deployment_with_upload_rules` → `make_deployment_with_release_upload_rules` to attach rules to the deployment's expanded `release.upload_rules`.
2. `agent/tests/sync/deployments.rs`: rename + rework `stores_upload_rules_from_expanded_deployment` → `stores_upload_rules_from_expanded_release` (rules ride the release); rework `upload_rules_not_expanded_error` to use a deployment with an expanded release whose `upload_rules` is `None`.
3. `agent/tests/models/deployment.rs`: remove the two `upload_rules: Some(vec![])` lines (857, 905).
4. `agent/tests/sync/syncer.rs`: remove the three `upload_rules: Some(vec![])` lines (346, 956, 1048).
5. Leave `agent/tests/sync/errors.rs`, `agent/tests/mocks/http_client.rs`, `agent/tests/models/upload_rule.rs`, and the `agent/tests/storage/*` upload-rule tests unchanged; confirm they still pass.
6. Commit S3: `test(sync): cover upload-rule acquisition via the release expansion`.

## Concrete Steps

Run all commands from `/home/ben/miru/workbench4/repos/agent` unless noted. Stay on branch `feat/uploads-via-deployment-expansion` (push mode).

S0 (gate):

    cd /home/ben/miru/workbench4/repos/agent/api && npx --yes @openapitools/openapi-generator-cli version
    cd /home/ben/miru/workbench4/repos/agent && grep -n upload_rules agent/tests/mocks/http_client.rs   # no hits

S1 (re-vendor + regen + commit) — see Plan of Work S1 for the stamping detail:

    api/regen.sh
    cargo build -p backend-api 2>&1 | tail -20
    git add api/specs/backend/v04.yaml libs/backend-api/src/models
    git commit -m "feat(api): re-vendor uploads contract from openapi 97809d8 (#151) — upload rules via release expansion"

S2 (rewire + commit):

    # edit agent/src/sync/deployments.rs + agent/src/sync/errors.rs per Plan of Work S2
    grep -rn 'store_expanded_upload_rules\|backend_dpl.upload_rules' agent/src   # expect none
    cargo build --features test 2>&1 | tail -20                                  # builds after S2 (source) — tests still need S3
    git add agent/src/sync && git commit -m "refactor(sync): acquire upload rules via the release expansion"

S3 (tests + commit):

    # edit the test sites per Plan of Work S3
    scripts/test.sh
    git add agent/tests && git commit -m "test(sync): cover upload-rule acquisition via the release expansion"

Targeted test runs (expect all green):

    cargo test --features test sync::deployments   # stores_upload_rules_from_expanded_release + upload_rules_not_expanded_error + existing suite
    cargo test --features test sync::syncer        # real-syncer tests (literals fixed)
    cargo test --features test models              # models/upload_rule.rs + models/deployment.rs
    cargo test --features test storage             # upload_rules round-trip + layout/caches unchanged

Expected: `sync::deployments` includes a passing `stores_upload_rules_from_expanded_release` (asserts `upl_rule_1`/`upl_rule_2` cached) and a passing `upload_rules_not_expanded_error` (asserts `SyncErr::UploadRulesNotExpanded`).

## Validation and Acceptance

**Changes MUST NOT be published until `scripts/preflight.sh` reports `Preflight clean`.** Run from the repo root:

    cd /home/ben/miru/workbench4/repos/agent
    scripts/update-deps.sh        # refresh Cargo.lock before linting
    scripts/preflight.sh          # lint + clippy -D warnings + fmt + machete/diet + audit + full test suite + covgate thresholds
    # final line MUST be: Preflight clean

Acceptance (human-verifiable):

1. **S1:** `api/specs/backend/v04.yaml` shows `version: v0.5.0-pre`, `x-release-version: v0.5.0-pre`, `x-git-commit.sha = 97809d89ad2ad29405859e5709b0a29cd72ecc01`; the `Release` schema has an `upload_rules` array, the `Deployment` schema has NO `upload_rules`, and neither `DeploymentExpansion` nor `DeploymentListExpansion` contains `upload_rules`/`DEPLOYMENT_*_UPLOAD_RULES`; no `$API_VERSION$` remains. The structural diff vs. the prior spec is exactly the five changes in S1 step 3. `libs/backend-api/src/models/release.rs` has `upload_rules`; `deployment.rs` does not; `cargo build -p backend-api` succeeds; `api_version.rs` renders `v0.5.0-pre`.
2. **S2:** `grep -rn 'backend_dpl.upload_rules\|store_expanded_upload_rules' agent/src` returns nothing; `agent/src/sync/deployments.rs` requests `release.upload_rules` and caches rules inside `store_expanded_release`; `cargo build --features test` succeeds after S2+S3.
3. **S3:** `cargo test --features test sync::deployments` passes, including `stores_upload_rules_from_expanded_release` (rules cached from the expanded release) and `upload_rules_not_expanded_error` (expanded release with `upload_rules: None` → `UploadRulesNotExpanded`). After a sync, `storage.upload_rules` is populated from the expanded release (no separate HTTP call) and persisted at `/var/lib/miru/resources/upload_rules.json`; nothing consumes the rules further (scope boundary respected).
4. `scripts/preflight.sh` prints `Preflight clean`.

Coverage expectations: `models` stays 100% (model files unchanged); `storage` untouched; `sync` (≥93.63) keeps its two upload-rule branches covered by the reworked `stores_upload_rules_from_expanded_release` (happy path: extract + `write_if_absent`) and `upload_rules_not_expanded_error` (the `ok_or_else` error branch). If covgate dips, add assertions — do NOT lower the threshold.

## Idempotence and Recovery

- **S1 is regenerative.** Re-running `api/regen.sh` reproduces `libs/backend-api/src/models/` from the vendored spec; safe to run repeatedly. Never hand-edit generated files — fix the spec and re-run.
- If `openapi-generator-cli` is unavailable, the plan is blocked at S0; STOP and report (do not hand-write models).
- **The workspace does not build between S1 and S2** (`agent` source still references the removed `Deployment.upload_rules`). Expected. Build/test only after S2 (source) and S3 (tests) land.
- If `cargo build --features test` fails after S3 with `no field upload_rules on type ...Deployment`, a test literal was missed — the sites are `agent/tests/models/deployment.rs` (857, 905) and `agent/tests/sync/syncer.rs` (346, 956, 1048).
- If `stores_*` or `upload_rules_not_expanded_error` fail with `UploadRulesNotExpanded` unexpectedly, `make_backend_release` was not updated to default `upload_rules: Some(Vec::new())`, or the test's expanded release lacks the array.
- If many existing sync tests fail with `UploadRulesNotExpanded`, the new check is firing on release-less deployments — verify the upload-rule extraction is gated inside `store_expanded_release` (after the `backend_dpl.release` guard), not on every deployment.
- Rollback: `git -C /home/ben/miru/workbench4/repos/agent checkout -- api/specs/backend/v04.yaml libs/backend-api agent/src agent/tests` restores pre-change state (only if abandoning).

### Delivery note (informational — the orchestrator handles this)

The agent `origin/main` has advanced with `#91` (a status-enum macro change). A rebase of `feat/uploads-via-deployment-expansion` onto `main` will occur at delivery time. This plan does not perform that rebase; it is recorded here so the implementer is not surprised by a later rebase step.

---

Change note (2026-06-28): Initial draft. Refactors the completed `20260628-uploads-via-deployment-expansion.md` (#150) to match openapi `97809d8` (#151), which moved the upload-rule expansion from the `Deployment` object to the `Release` object. Re-vendors the contract (continuing `v0.5.0-pre` stamping), switches the syncer's expansion from `upload_rules` to `release.upload_rules`, and folds upload-rule extraction into `store_expanded_release` (mirroring `git_commit`), removing the standalone `store_expanded_upload_rules` helper. `UploadRulesNotExpanded` now fires when an expanded release lacks `upload_rules`. Stays on branch `feat/uploads-via-deployment-expansion` (push mode). Key risks flagged: the workspace won't build between regen and the source rewire; `make_backend_release` must default `upload_rules: Some(Vec::new())`; the upload-rule extraction must precede the git_commit early-return inside `store_expanded_release`.
