# Uploads: re-vendor openapi #150 and ride the deployment expansion for upload rules

This ExecPlan is a living document. The sections **Progress**, **Surprises & Discoveries**, **Decision Log**, and **Outcomes & Retrospective** MUST be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `/home/ben/miru/workbench4/repos/agent` (the Rust device agent — the only repo edited) | read-write | Re-vendor `api/specs/backend/v04.yaml` from openapi `#150` and regenerate `libs/backend-api` models; remove the now-obsolete standalone `http/upload_rules.rs` client + its tests/mock; wire upload-rule acquisition onto the existing deployment fetch (request the `upload_rules` expansion, extract + cache rules in `sync/deployments.rs`). |
| `/home/ben/miru/workbench4/repos/openapi` | read-only | Source of the vendored bundle. Read `apis/apps/backend-server/agent/openapi.gen.yaml` at `origin/main` commit `e0bc63e` (full `e0bc63ebe87f040582e4efbdb14d64e72bec5b55`). No edits. |

This plan lives in `plans/backlog/` per the agent repo's plan conventions (`plans/{active,archived,backlog,completed}/`). Commit all changes from inside the agent repo's own git context (see workbench `CLAUDE.md`), never from the workbench root. Work happens on branch `feat/uploads-via-deployment-expansion` (base `main`).

This plan implements exactly the four steps below and **deliberately ENDS at "the deployed release's upload rules arrive via the deployment expansion and are cached in local state."** Nothing consumes the rules beyond storage.

### Explicitly OUT OF SCOPE (do NOT implement here)

These are intentionally NOT in this plan and MUST NOT be implemented:

- File discovery / glob matching on `source.glob`; the per-rule poll loop (`poll_interval_secs`); stability / finalization detection (`stability_window_secs`) — **M2**.
- Streaming sha256 digest + size computation over candidate files — **M3**.
- `POST /uploads` (presigned `PUT`) and `POST /uploads/{upload_id}/confirm` — **M3**.
- The local uploads ledger / idempotency / retry state — **M4**.
- `delete_policy` enforcement (deleting local source files) — **M5**.
- Any background upload worker or `app/run.rs` integration — **M5**.

The agent already carries (from prior milestones, on this branch / `main`): `agent/src/models/upload_rule.rs` (domain model + `From<BaseUploadRule>` — **KEEP**), `agent/src/storage/upload_rules.rs` (persisted store, spawned in `storage/mod.rs` — **KEEP/reuse**). The generated POST/confirm models (`Upload`, `PresignedUpload`, `CreateUploadRequest`, `UploadSource`, `UploadDestination`, `UploadRequiredHeaders`, `UploadStatus`, `BaseUpload`) remain generated-but-unused — that is expected and fine.

## Purpose / Big Picture

openapi `#150` ("feat(agent): deliver upload rules via deployment expansion") changes how the agent learns its upload rules. Previously rules were fetched from a standalone `GET /upload_rules` (vendored at fe6e9ca, consumed by `agent/src/http/upload_rules.rs`). `#150` REMOVES that endpoint and instead delivers rules **as an expansion on the deployment object** — exactly like `config_instances` and `release`. The agent must therefore:

1. Re-vendor the contract to pick up the new shape.
2. Delete the obsolete standalone client (and its tests/mock plumbing).
3. Request the `upload_rules` expansion on the deployment fetch the syncer already performs, then extract + cache the rules from the deployment response.

This is the minimal, contained change that swaps the acquisition mechanism without touching any downstream upload behavior (all still out of scope).

**Observable outcome at completion:** after a sync, the deployed release's upload rules are present in the agent's local state at `/var/lib/miru/resources/upload_rules.json`, having ridden in on the deployment fetch (no separate HTTP call). `agent/src/http/upload_rules.rs` no longer exists, and `GET /upload_rules` is gone from the vendored spec. `scripts/preflight.sh` prints `Preflight clean`.

## Progress

- [ ] **S1** Re-vendor `api/specs/backend/v04.yaml` from openapi `e0bc63e`, preserving `v0.5.0-pre` stamping; run `api/regen.sh`; commit spec + regenerated `libs/backend-api/src/models/` atomically.
- [ ] **S2** Remove the obsolete standalone client (`http/upload_rules.rs` + registration) and its tests/mock plumbing.
- [ ] **S3** Wire acquisition via the deployment expansion (request `upload_rules`, extract + cache in `sync/deployments.rs`).
- [ ] **S4** Tests: sync expansion extraction + caching; update fixtures/factories; remove standalone-client tests.
- [ ] **V** `scripts/preflight.sh` reports `Preflight clean`.

Use timestamps when completing steps. Split partially-completed work into "done" / "remaining".

## Surprises & Discoveries

(Add entries as work proceeds. Seed findings from the verified context below.)

- **The vendored spec is already at `v0.5.0-pre` (commit fe6e9ca).** There is NO openapi agent release tag newer than `agent/v0.4.0`, so we CONTINUE the established `v0.5.0-pre` pre-release stamping rather than inventing a release version. The only `info:` changes vs. the current file are the three `x-git-commit` keys (sha/url/message) → `e0bc63e`.
- **Verified spec diff (current fe6e9ca → e0bc63e), via structural YAML comparison — exactly `#150`, no surprises:**
  - `paths` removed: `/upload_rules` (the standalone `getUploadRules` GET).
  - `components.schemas` removed: `UploadRuleList`.
  - `components.schemas` changed: `APIVersion` (the `$API_VERSION$` placeholder → `v0.5.0-pre`, a vendoring substitution, not a contract change), `Deployment` (gains `upload_rules: array<BaseUploadRule>`), `DeploymentExpansion` (gains enum `upload_rules` / varname `DEPLOYMENT_EXPAND_UPLOAD_RULES`), `DeploymentListExpansion` (gains `DEPLOYMENT_LIST_EXPAND_UPLOAD_RULES`).
  - Unchanged: `BaseUploadRule`, `UploadRuleSource`, `UploadRuleDestination`, `UploadDeletePolicy` (still reached via the expansion). No params added/removed. No OTHER agent-bundle changes since fe6e9ca. **Re-verify this diff after writing the new spec; flag any additional change for review before regen.**
- **The raw `e0bc63e` bundle has `info.version: 0.0.0` and two `$API_VERSION$` placeholders** (the `APIVersion` enum value at line ~418 and the `MiruVersion` parameter `example:` at line ~1753). The openapi release pipeline substitutes these at stamping time; the prior vendor reproduced this by hand. We do the same → `v0.5.0-pre`, so `ApiVersion::API_VERSION` (and therefore the outbound `Miru-Version` header set in `agent/src/http/request.rs`) renders `v0.5.0-pre`, identical to today. No header behavior change.
- **The syncer builds its deployment expansions list as raw `&str` literals** in `agent/src/sync/deployments.rs::fetch_active_deployments` (line 122): `let expansions: &[&str] = &["config_instances", "release.git_commit"];`. It does NOT use the generated `DeploymentExpansion` enum, so picking up the new enum variant requires NO code change — only appending the `"upload_rules"` string. This is the exact, unambiguous place Step 3 edits.
- **`config_instances` extraction pattern to mirror** (`pull_deployments`, lines 89-108): each active deployment is required to carry the expansion — `backend_dpl.config_instances.clone().ok_or_else(|| SyncErr::CfgInstsNotExpanded(CfgInstsNotExpandedErr { deployment_id }))?` — then it iterates the array writing each item with `storage.cfg_insts.meta.write_if_absent(id, value, |_, _| false)`. The `store_expanded_release` helper (lines 238-265) shows the same `write_if_absent` idempotent-write pattern for immutable expanded data (releases/git_commits). Upload rules are immutable (digest-deduped), so they mirror `store_expanded_release`'s `write_if_absent` exactly.
- **CRITICAL build ordering:** regen (S1) DELETES `libs/backend-api/src/models/upload_rule_list.rs` and drops `pub mod upload_rule_list;` / `pub use ...UploadRuleList` from the generated `models/mod.rs`. Every file that imports `backend_api::models::UploadRuleList` then FAILS to compile: `agent/src/http/upload_rules.rs`, `agent/tests/http/upload_rules.rs`, and `agent/tests/mocks/http_client.rs`. So S2 (removal) is NOT optional cleanup — the workspace will not build between S1 and S2. Do S1 and S2 together (or S2 immediately after S1) before attempting a full build/test.
- **CRITICAL test regression risk:** after S3, `pull_deployments` will require the `upload_rules` expansion on every active deployment (mirroring `config_instances`). The `make_deployment` factory in `agent/tests/sync/helpers.rs` (line 30) currently leaves `upload_rules` at `Default` (`None`), which would make EVERY existing sync test trip the new not-expanded error. `make_deployment` MUST default `upload_rules: Some(Vec::new())` (like it already does `config_instances: Some(...)`), and the dedicated not-expanded test sets it back to `None` explicitly.

## Decision Log

- **Decision: Continue `v0.5.0-pre` pre-release stamping (do NOT invent a release version).** No openapi agent release tag is newer than `agent/v0.4.0`; the contract is still pre-release. Only the `x-git-commit` block moves to `e0bc63e`. Rationale: matches the established convention from the fe6e9ca/4c92b71 vendors; the `-pre` suffix marks the snapshot as not-yet-released. Date/Author: 2026-06-28 / plan author.
- **Decision: Treat a missing `upload_rules` expansion as a hard sync error**, mirroring `CfgInstsNotExpanded`. Add an `UploadRulesNotExpandedErr` variant to `agent/src/sync/errors.rs` and `?`-propagate it in `pull_deployments`. Rationale: the syncer explicitly requests `expand=upload_rules`, so the backend always returns the array (possibly empty); a `None` means a contract violation, exactly as for `config_instances`. The faithful mirror also gives a clean covering test (the existing `cfg_inst_not_expanded_error` is the template). ALTERNATIVE considered: treat `None` as empty (`unwrap_or_default`) — rejected because it silently masks a broken backend and diverges from the established `config_instances` contract. If the implementer finds the backend legitimately omits the array, revisit here. Date/Author: 2026-06-28 / plan author.
- **Decision: Extract + cache rules in a dedicated `store_expanded_upload_rules` helper** mirroring `store_expanded_release` (immutable data → `write_if_absent(id, rule, |_, _| false)`), called from the `pull_deployments` loop. Rationale: keeps the immutable-expansion caching pattern consistent across releases/git_commits/upload_rules; `write_if_absent` avoids per-sync I/O for already-cached rules. Date/Author: 2026-06-28 / plan author.

## Outcomes & Retrospective

(Fill in after implementation: generated model files that changed, the final spec diff, covgate numbers, commit list, and any integration-test gating.)

## Context and Orientation

The agent is a Rust workspace rooted at `repos/agent/Cargo.toml`: `agent/` (binary crate — all logic), `libs/backend-api/` and `libs/device-api/` (OpenAPI-generated models; do NOT hand-edit — regenerate via `api/regen.sh`). Repo conventions: `repos/agent/AGENTS.md` (import ordering, `thiserror` errors, `#[cfg(feature = "test")]` gating, `scripts/test.sh` with `--features test`, per-module `.covgate`).

### Vendoring & codegen surface (S1)

- `api/specs/backend/v04.yaml` — the vendored agent bundle. Current `info:` block (lines 2-13): `version: v0.5.0-pre`, `license`, `x-release-version: v0.5.0-pre`, `x-git-commit:{sha,url,message}` pinned to fe6e9ca.
- `api/Makefile` `gen-backend` runs `npx --yes @openapitools/openapi-generator-cli generate -i specs/backend/v04.yaml -g rust -t templates/rust -o codegen/backend --additional-properties=packageName=backend-api`.
- `api/regen.sh` runs `make gen`, then wipes `libs/backend-api/src/models/*` + `libs/device-api/src/models/*` and copies freshly generated models in. **It regenerates `libs/backend-api/src/models/mod.rs` too** (so the `upload_rule_list` module/export disappears automatically).
- `api/templates/rust/model.mustache` — custom model template (forward-compatible enums). Codegen uses it; do not bypass it.
- `api/openapitools.json` — pins the generator CLI version.

Generated-model effects of the `e0bc63e` diff (verify after regen):
- `libs/backend-api/src/models/deployment.rs` — `Deployment` gains `#[serde(rename = "upload_rules", skip_serializing_if = "Option::is_none")] pub upload_rules: Option<Vec<models::BaseUploadRule>>` (mirrors the existing `config_instances: Option<Vec<models::ConfigInstance>>`).
- `libs/backend-api/src/models/deployment_expansion.rs` — gains `DEPLOYMENT_EXPAND_UPLOAD_RULES` (renders `"upload_rules"`).
- `libs/backend-api/src/models/deployment_list_expansion.rs` — gains `DEPLOYMENT_LIST_EXPAND_UPLOAD_RULES`.
- `libs/backend-api/src/models/upload_rule_list.rs` — **removed**; `models/mod.rs` drops its `pub mod`/`pub use`.
- `libs/backend-api/src/models/base_upload_rule.rs` (and `upload_rule_source.rs`, `upload_rule_destination.rs`, `upload_delete_policy.rs`) — **unchanged**.

### Read-path code touched (S2–S3)

- `agent/src/http/upload_rules.rs` — standalone `list`/`list_all` client over `GET /upload_rules`. **OBSOLETE → delete.** Referenced only by `agent/src/http/mod.rs:12` (`pub mod upload_rules;`) and the test files below. NOT referenced by `sync`.
- `agent/src/http/mod.rs` — remove line 12 (`pub mod upload_rules;`).
- `agent/src/sync/deployments.rs` — `fetch_active_deployments` (expansions list, line 122), `pull_deployments` (config_instances extraction, lines 89-108), `store_expanded_release` (immutable `write_if_absent` template, lines 238-265), the `Storage<'a>` struct (lines 27-32). `apply_storage` (lines 35-43) does NOT need upload_rules.
- `agent/src/sync/syncer.rs::sync_impl` (lines 236-241) — the only production construction site of `deployments::Storage`.
- `agent/src/sync/errors.rs` — `CfgInstsNotExpandedErr` (lines 65-71) + `SyncErr` enum (line 98) + `From` impl (lines 137-141) + `impl_error!` list (line 155) — the template for the new `UploadRulesNotExpandedErr`.
- `agent/src/storage/upload_rules.rs`, `agent/src/storage/mod.rs` (`UploadRules` field already on `Storage`, spawned line 143-145, shut down line 202), `agent/src/storage/layout.rs` (`upload_rules()` → `resources/upload_rules.json`, lines 65-66) — **already wired; reuse as-is.**
- `agent/src/models/upload_rule.rs` — domain model + `From<backend_client::BaseUploadRule>` — **KEEP unchanged.**

### Test surface (S2, S4)

- `agent/tests/http/upload_rules.rs` + `agent/tests/http/mod.rs:10` — standalone-client tests. **Delete + unregister.**
- `agent/tests/mocks/http_client.rs` — remove the standalone upload_rules plumbing: `UploadRuleList` import (line 8), `Call::ListUploadRules` (line 35), `ListUploadRulesFn` type (line 58), `list_upload_rules_fn` field (line 74) + its init (line 92), `set_list_all_upload_rules`/`set_list_upload_rules_page` (lines 169-188), the `GET /upload_rules` route (line 230), and the `Call::ListUploadRules` response arm (lines 263-265). Drop the now-unused `BaseUploadRule` import from this file IF the compiler flags it unused (it is used only by the removed setter). The deployment-expansion fixtures live in `helpers.rs`, not the mock.
- `agent/tests/sync/helpers.rs` — factories (`make_deployment` line 30, `make_deployment_with_release` line 76, `make_backend_release`/`make_backend_git_commit`). Add an upload-rule factory + an `assert_upload_rule_stored` helper; update `make_deployment` to default `upload_rules: Some(Vec::new())`.
- `agent/tests/sync/deployments.rs` — `Fixture` (lines 28-101) constructs `deployments::Storage` directly (line 81); existing expansion tests `stores_release_and_git_commit_from_expanded_deployment` (line 172) and `cfg_inst_not_expanded_error` (line 431) are the mirrors.

### Validation tooling

- `scripts/preflight.sh` runs, in parallel: `scripts/lint.sh` (import linter, `cargo fmt`, `cargo machete`/diet, security audit, clippy `-D warnings`), and `scripts/covgate.sh` (tests + per-module coverage gate). Prints `Preflight clean` on success.
- Relevant `.covgate` minimums: `agent/src/models/` = **100**, `agent/src/http/` = **93.9**, `agent/src/storage/` = **94.83**, `agent/src/sync/` = **93.63**.
- `scripts/test.sh` = `RUST_LOG=off cargo test --features test` (the `--features test` flag is REQUIRED; mocks/helpers are behind it).
- Run `scripts/update-deps.sh` before linting to refresh `Cargo.lock`.

## Plan of Work

### S0 — Preflight the generator (gate)

Confirm the generator can run before changing anything:

    cd /home/ben/miru/workbench4/repos/agent/api
    npx --yes @openapitools/openapi-generator-cli version

If this fails (no network / npx / Java unavailable), **STOP and report**. Do NOT hand-write or hand-edit generated models in `libs/backend-api/src/models/` — `regen.sh` regenerates them wholesale and any hand-edit is silently destroyed and diverges from the contract. Reporting the blocker is the correct outcome.

### S1 — Re-vendor spec + regenerate models (one atomic commit)

1. Extract the source bundle (read-only) from openapi and capture the commit metadata:

       cd /home/ben/miru/workbench4/repos/openapi
       git show origin/main:apis/apps/backend-server/agent/openapi.gen.yaml > /tmp/.../scratchpad/bundle-e0bc63e.yaml
       git show -s --format='%H%n%s' e0bc63e
       # full sha:  e0bc63ebe87f040582e4efbdb14d64e72bec5b55
       # subject:   feat(agent): deliver upload rules via deployment expansion (#150)

2. Produce the stamped `api/specs/backend/v04.yaml`. The ONLY differences from the raw `e0bc63e` bundle are the vendoring stamp (mirror the current file exactly):
   - `info.version: 0.0.0` → `version: v0.5.0-pre`
   - keep the existing `license:` block (already matches)
   - `info.x-release-version: v0.5.0-pre`
   - `info.x-git-commit:` → `sha: e0bc63ebe87f040582e4efbdb14d64e72bec5b55`, `url: https://github.com/mirurobotics/openapi/commit/e0bc63ebe87f040582e4efbdb14d64e72bec5b55`, `message: 'feat(agent): deliver upload rules via deployment expansion (#150)'`
   - substitute BOTH `$API_VERSION$` placeholders (the `APIVersion` enum value and the `MiruVersion` parameter `example:`) → `v0.5.0-pre`
   - leave everything else (servers/security/paths/components from `e0bc63e`) exactly as the source bundle has it.

   Easiest faithful approach: copy the current `v04.yaml` `info:` block (lines 2-13) onto the new bundle's body, update only the three `x-git-commit` keys, and do the two `$API_VERSION$` substitutions.

3. **Re-verify the diff before regen** (must match the Surprises entry exactly):

       cd /home/ben/miru/workbench4/repos/agent
       grep -nE '^  version:|x-release-version:|sha:|\$API_VERSION\$' api/specs/backend/v04.yaml   # version/release = v0.5.0-pre; no $API_VERSION$ left
       grep -nE '/upload_rules|UploadRuleList' api/specs/backend/v04.yaml                          # expect NONE
       grep -nE 'DEPLOYMENT_EXPAND_UPLOAD_RULES|DEPLOYMENT_LIST_EXPAND_UPLOAD_RULES' api/specs/backend/v04.yaml  # expect present

   Compare paths + schema names against the pre-change spec; if anything beyond `{paths: -/upload_rules}`, `{schemas: -UploadRuleList}`, `{schemas changed: APIVersion, Deployment, DeploymentExpansion, DeploymentListExpansion}` differs, **flag it for review** before continuing.

4. Regenerate and build:

       api/regen.sh
       cargo build -p backend-api 2>&1 | tail -20
       ls libs/backend-api/src/models | grep -i upload          # upload_rule_list.rs gone; base_upload_rule.rs etc. remain
       grep -n upload_rules libs/backend-api/src/models/deployment.rs            # Deployment.upload_rules field present
       grep -n UPLOAD_RULES libs/backend-api/src/models/deployment_expansion.rs  # enum variant present

   `cargo build -p backend-api` should succeed (generated crate only). The full workspace build will FAIL here because `agent`/tests still import the removed `UploadRuleList` — that is expected and fixed by S2.

5. Confirm the `Miru-Version` consequence is a no-op:

       grep -n 'rename\|API_VERSION' libs/backend-api/src/models/api_version.rs   # renders v0.5.0-pre (unchanged from today)

6. Commit S1 atomically (spec + regenerated `libs/backend-api/src/models/`) so the generated churn is isolated:
   `feat(api): re-vendor uploads contract from openapi e0bc63e (#150) — upload rules via deployment expansion`.

### S2 — Remove the obsolete standalone client

1. `git rm agent/src/http/upload_rules.rs`; remove `pub mod upload_rules;` (line 12) from `agent/src/http/mod.rs`.
2. `git rm agent/tests/http/upload_rules.rs`; remove `pub mod upload_rules;` (line 10) from `agent/tests/http/mod.rs`.
3. In `agent/tests/mocks/http_client.rs` remove the standalone plumbing listed in the Test surface section (import, `Call::ListUploadRules`, `ListUploadRulesFn`, field + init, both setters, route, response arm; drop the `BaseUploadRule` import if the compiler reports it unused).
4. Re-verify nothing else references the removed client:

       grep -rn 'http::upload_rules\|ListUploadRules\|UploadRuleList\|set_list_.*upload_rules' agent/src agent/tests
       # expect: NO hits (the only remaining `upload_rules` hits are storage::UploadRules, models::UploadRule(s), layout)
       cargo build --features test 2>&1 | tail -20   # workspace builds again after S1+S2

   **KEEP** `agent/src/models/upload_rule.rs` and `agent/src/storage/upload_rules.rs` untouched.

### S3 — Wire acquisition via the deployment expansion

1. **Request the expansion** — `agent/src/sync/deployments.rs::fetch_active_deployments`, line 122:

       let expansions: &[&str] = &["config_instances", "release.git_commit", "upload_rules"];

2. **Add the storage field** to `sync::deployments::Storage<'a>` (lines 27-32):

       pub upload_rules: &'a storage::UploadRules,

   `apply_storage` (lines 35-43) does NOT need it — leave unchanged.

3. **Add the not-expanded error** in `agent/src/sync/errors.rs`, mirroring `CfgInstsNotExpandedErr`:

       #[derive(Debug, thiserror::Error)]
       #[error("deployment '{deployment_id}' did not have upload_rules expansion (backend did not expand upload rules)")]
       pub struct UploadRulesNotExpandedErr { pub deployment_id: String }
       impl crate::errors::Error for UploadRulesNotExpandedErr {}

   Add `UploadRulesNotExpanded(UploadRulesNotExpandedErr)` to the `SyncErr` enum, a `From` impl, and the `impl_error!` list entry — all mirroring `CfgInstsNotExpanded`.

4. **Extract + cache** — add a `store_expanded_upload_rules` helper mirroring `store_expanded_release`, and call it inside the `pull_deployments` loop (after `store_expanded_release`):

       async fn store_expanded_upload_rules(
           storage: &Storage<'_>,
           backend_dpl: &backend_client::Deployment,
       ) -> Result<(), SyncErr> {
           let rules = backend_dpl.upload_rules.clone().ok_or_else(|| {
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
           Ok(())
       }

   `write_if_absent` matches the immutable-data pattern of `store_expanded_release` (rules are digest-deduped/immutable). Milestone ENDS here — no consumer reads `storage.upload_rules` beyond this cache.

5. **Production construction site** — `agent/src/sync/syncer.rs::sync_impl` (lines 236-241): add `upload_rules: storage_ref.upload_rules.as_ref(),` to the `deployments::Storage { ... }` literal (the `storage::Storage` already owns `upload_rules: Arc<UploadRules>`).

### S4 — Tests

Tests use `--features test` (run via `scripts/test.sh`); files mirror `agent/src/` under `agent/tests/`. No `#[serial]` needed (storage tests use temp dirs).

1. **Factories** — `agent/tests/sync/helpers.rs`:
   - Add a backend upload-rule factory (mirror `make_backend_release`):

         pub fn make_backend_upload_rule(id: &str) -> backend_api::models::BaseUploadRule {
             backend_api::models::BaseUploadRule {
                 id: id.to_string(),
                 created_at: Utc::now().to_rfc3339(),
                 updated_at: Utc::now().to_rfc3339(),
                 ..Default::default()   // object defaults to UploadRule; source/destination boxed defaults
             }
         }

     (`BaseUploadRule` derives `Default`; `object` defaults to `Object::UploadRule`. Setting valid rfc3339 timestamps keeps the `From` parse off the UNIX_EPOCH fallback.)
   - **Update `make_deployment` (line 30)** to default `upload_rules: Some(Vec::new())` so existing tests don't trip the new required expansion. Add an optional helper `make_deployment_with_upload_rules(id, cfg_inst_args, rule_ids: &[&str])` (or set the field on a deployment built by `make_deployment`) for the positive test.
   - Add `assert_upload_rule_stored(upload_rule_stor: &UploadRules, id: &str)` mirroring `assert_release_stored`; import `miru_agent::storage::UploadRules`.

2. **Fixture** — `agent/tests/sync/deployments.rs` (lines 28-101): add a `upload_rule_stor: UploadRules` field, spawn it in `Fixture::new` (`UploadRules::spawn(16, dir.file("upload_rules.json"), 1000)`), and pass `upload_rules: &self.upload_rule_stor` into the `deployments::Storage { ... }` literal in `Fixture::sync` (line 81). Import `UploadRules` in the test module.

3. **New sync tests** (in `mod pull_success` / alongside `cfg_inst_not_expanded_error`):
   - `stores_upload_rules_from_expanded_deployment` — build a deployment with `upload_rules: Some(vec![make_backend_upload_rule("upl_rule_1"), make_backend_upload_rule("upl_rule_2")])`, set the mock via `set_list_all_deployments`, run `sync`, assert both rules are cached via `assert_upload_rule_stored`. Mirrors `stores_release_and_git_commit_from_expanded_deployment` (line 172). **Covers the happy-path extraction + `write_if_absent` branch.**
   - `upload_rules_not_expanded_error` — build a deployment with `upload_rules: None` (everything else valid), run `sync`, assert the error is `SyncErr::UploadRulesNotExpanded(_)` (or a `SyncErrors` containing it). Mirrors `cfg_inst_not_expanded_error` (line 431). **Covers the new `ok_or_else` error branch** (the only genuinely new branch this change adds in `sync`).

4. **Remove** the standalone-client tests (done in S2): `agent/tests/http/upload_rules.rs` + its registration.

5. Confirm the existing `agent/tests/storage/upload_rules.rs` round-trip and `agent/tests/storage/layout.rs`/`caches.rs` upload-rules tests still pass unchanged (storage wiring is untouched).

## Test Steps

    cd /home/ben/miru/workbench4/repos/agent
    cargo build --features test                 # builds clean only after S1+S2 together
    scripts/test.sh                             # RUST_LOG=off cargo test --features test
    # Targeted:
    cargo test --features test sync::deployments # new expansion tests + existing suite (all green)
    cargo test --features test storage           # upload_rules round-trip + layout/caches unchanged
    cargo test --features test models            # models/upload_rule.rs From/Deserialize still covered

New/changed branches and their covering tests:
- `store_expanded_upload_rules` happy path (extract + `write_if_absent`) → `stores_upload_rules_from_expanded_deployment`.
- `UploadRulesNotExpanded` error path → `upload_rules_not_expanded_error`.
- `From<BaseUploadRule>` (and the `DeletePolicy`/timestamp-fallback branches) stays covered by the existing `agent/tests/models/upload_rule.rs` — it is now exercised through the sync path too, but its dedicated model tests are what keep `models` at 100%; do NOT delete them.

Coverage expectations:
- `models` (=100): unchanged file, unchanged tests → still 100%.
- `http` (=93.9): removing the dead standalone client + its tests removes covered-but-unused surface; coverage should be neutral-to-up. Verify it does not dip (it shouldn't — both code and tests are removed together).
- `storage` (=94.83): untouched.
- `sync` (=93.63): two new branches, both covered by the new tests above. If covgate dips, add assertions (do not lower the threshold).

## Validation and Acceptance

**Changes MUST NOT be published until `scripts/preflight.sh` reports `Preflight clean`.** Run from the repo root:

    cd /home/ben/miru/workbench4/repos/agent
    scripts/update-deps.sh        # refresh Cargo.lock before linting
    scripts/preflight.sh          # lint + clippy -D warnings + fmt + cargo-diet/machete + audit + full test suite + covgate.sh thresholds
    # final line MUST be: Preflight clean

Acceptance (human-verifiable):

1. **S1:** `api/specs/backend/v04.yaml` shows `version: v0.5.0-pre`, `x-release-version: v0.5.0-pre`, `x-git-commit.sha = e0bc63ebe87f040582e4efbdb14d64e72bec5b55`; contains `DEPLOYMENT_EXPAND_UPLOAD_RULES` and `Deployment.upload_rules`; contains NO `/upload_rules` path or `UploadRuleList` schema; no `$API_VERSION$` placeholder remains. The structural diff vs. the prior spec is exactly the four schema changes + two removals listed in Surprises (nothing else). `libs/backend-api/src/models/` has no `upload_rule_list.rs`; `cargo build -p backend-api` succeeds; `api_version.rs` renders `v0.5.0-pre`.
2. **S2:** `agent/src/http/upload_rules.rs` and `agent/tests/http/upload_rules.rs` are gone; `grep -rn 'http::upload_rules\|UploadRuleList\|ListUploadRules' agent/` returns nothing; `cargo build --features test` succeeds.
3. **S3/S4:** `cargo test --features test sync::deployments` passes, including `stores_upload_rules_from_expanded_deployment` (rules cached from the deployment expansion) and `upload_rules_not_expanded_error`. After a sync, `storage.upload_rules` is populated from the deployment response (no `GET /upload_rules` call) and persisted at `/var/lib/miru/resources/upload_rules.json`; nothing consumes the rules further (scope boundary respected).
4. `scripts/preflight.sh` prints `Preflight clean`.

## Idempotence and Recovery

- **S1 is regenerative.** Re-running `api/regen.sh` reproduces `libs/backend-api/src/models/` from the vendored spec; safe to run repeatedly. Never hand-edit generated files — fix the spec and re-run.
- If `openapi-generator-cli` is unavailable, the plan is blocked at S0; STOP and report (do not hand-write models).
- **The workspace does not build between S1 and S2** (removed `UploadRuleList` is still imported). This is expected; do S1+S2 as a pair before any full build/test. Commit them separately (generated churn vs. hand-written removal) but verify the build only after both.
- If `cargo build --features test` fails after S3 with a missing-field error on `deployments::Storage`, a construction site was missed — there are exactly two (`agent/src/sync/syncer.rs:236` and `agent/tests/sync/deployments.rs:81`).
- If many pre-existing sync tests suddenly fail with `UploadRulesNotExpanded`, the `make_deployment` factory default was not updated to `upload_rules: Some(Vec::new())` (see Surprises).
- Rollback: `git -C /home/ben/miru/workbench4/repos/agent checkout -- api/specs/backend/v04.yaml libs/backend-api agent/src agent/tests` restores pre-change state (only if abandoning).

---

Change note (2026-06-28): Initial draft. Re-vendors openapi `e0bc63e` (#150 — upload rules delivered via the deployment expansion; standalone `GET /upload_rules` + agent-local `UploadRuleList` removed), continuing `v0.5.0-pre` pre-release stamping. Removes the obsolete `http/upload_rules.rs` client + its tests/mock plumbing, and switches upload-rule acquisition to ride the existing deployment fetch (append `"upload_rules"` to the syncer's expansions list at `sync/deployments.rs:122`; extract + `write_if_absent`-cache via a new `store_expanded_upload_rules` mirroring `store_expanded_release`; new `UploadRulesNotExpanded` error mirroring `CfgInstsNotExpanded`). Key risks flagged: the workspace won't build between regen and client-removal (do them as a pair), and `make_deployment` must default `upload_rules: Some(Vec::new())` or every existing sync test trips the new required expansion.
