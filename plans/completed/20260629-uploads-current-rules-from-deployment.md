# Uploads worker: act on the CURRENT active upload rules (deployment → release → rules)

This ExecPlan is a living document. The sections **Progress**, **Surprises & Discoveries**, **Decision Log**, and **Outcomes & Retrospective** MUST be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `/home/ben/miru/workbench4/repos/agent` (the Rust device agent — the only repo edited) | read-write | (1) Add `upload_rule_ids: Vec<models::UploadRuleID>` to the domain `Release` (`agent/src/models/release.rs`), mirroring `Deployment.config_instance_ids`, and populate it during acquisition in `agent/src/sync/deployments.rs::store_expanded_release`. (2) Change the upload-discovery worker (`agent/src/workers/uploads.rs`) so it sources its active rule set by traversal — current `Deployed` deployment → its `release_id` → that release's `upload_rule_ids` → rule BODIES from `storage::UploadRules` by id — instead of reading the entire append-only `upload_rules.values()` union. Thread the `deployments` + `releases` stores into `uploads::run`/`run_impl` and the `init_uploads_worker` wiring in `agent/src/app/run.rs`. No pruning: the append-only `storage::UploadRules` store stays the by-id BODY lookup. |

Branch: `feat/uploads-file-discovery` (PR #93; push mode — stay on this branch). Base: `main`. This plan lives in `plans/backlog/` per the agent repo's plan conventions (`plans/{active,archived,backlog,completed}/`). Commit all changes from inside the agent repo's own git context (workbench `CLAUDE.md`), never from the workbench root.

### Explicitly OUT OF SCOPE

- **No pruning / no store deletion** (explicit user decision). `storage::UploadRules` remains append-only — it accumulates rule bodies from every deployment and stays the by-id BODY cache. We only stop *trusting* `.values()` as the active set; we never delete from it.
- No digest/sha256, no `POST /uploads`/presigned PUT/confirm, no ledger persistence, no `delete_policy` enforcement, no outbound HTTP. The placeholder `info!` sink (M2) is unchanged. Glob matching, per-rule cadence, the stability state machine, `decide_ready`, the log sink, and the in-memory dedupe are all UNCHANGED — only the *rule sourcing* in `run_impl` changes.

## Purpose / Big Picture

The append-only `storage::UploadRules` cache accumulates upload-rule bodies from **every** deployment ever synced (`store_expanded_release` uses `write_if_absent` and never deletes — verified `agent/src/sync/deployments.rs:270-277`). The discovery worker currently reads the WHOLE union via `upload_rules.values().await` (`agent/src/workers/uploads.rs:93`), so a rule that belonged to a since-replaced release keeps being collected forever. That is the bug: stale rules keep being acted on.

The fix resolves the **active** rule set by traversal, mirroring how the rest of the agent treats "what is currently deployed":

1. **Current deployment** — the deployment(s) with `activity_status == DplActivity::Deployed` in `storage::Deployments`.
2. **Its release** — `storage::Releases.read_optional(deployment.release_id)`.
3. **That release's upload rules** — `release.upload_rule_ids`, then fetch each BODY from `storage::UploadRules` by id (skip + `debug!` any id missing from the store).

The crux is step (3)'s missing linkage: the domain `Release` today has NO rule linkage (`{id, version, git_commit_id, created_at, updated_at}` only — `agent/src/models/release.rs:14-21`), even though the backend release carries `upload_rules: Option<Vec<BaseUploadRule>>` (`libs/backend-api/src/models/release.rs:37`). We add `upload_rule_ids` to the domain `Release` and populate it during acquisition, exactly mirroring how `Deployment.config_instance_ids` is carried from the backend deployment.

**Observable outcome at completion:** with the worker enabled, only rules belonging to the currently-`Deployed` deployment's release are scanned/acted on. A rule body present in the append-only store but NOT in the current release's `upload_rule_ids` is never acted on. With no `Deployed` deployment the worker idles (empty active set). `scripts/preflight.sh` prints `Preflight clean`.

## Progress

- [x] **M1** (2026-06-29) Added `upload_rule_ids: Vec<UploadRuleID>` to domain `Release` (field + `Default` empty `Vec` + `#[serde(default)]` `Deserialize` path) and replaced `From<backend_client::Release>` with `Release::from_backend(release, upload_rule_ids)` mirroring `Deployment::from_backend`.
- [x] **M2** (2026-06-29) Populated `upload_rule_ids` in `store_expanded_release` (extract ids from the expanded `backend_release.upload_rules` FIRST, then build via `Release::from_backend`). Rule BODIES still written append-only via `write_if_absent` — unchanged.
- [x] **M3** (2026-06-29) Worker rule sourcing in `uploads.rs::run_impl` now resolves the active set by traversal via a new `active_upload_rules` helper (Deployed deployment → release → `upload_rule_ids` → bodies by id; union+dedupe; skip + `debug!` missing ids; cache errors logged + treated as empty). Threaded `deployments` + `releases` stores through `run`/`run_impl`.
- [x] **M4** (2026-06-29) Wired the two extra stores into `init_uploads_worker` (`agent/src/app/run.rs`).
- [x] **M5** (2026-06-29) Tests — sync linkage test; 9 direct `active_upload_rules` unit tests (resolved/stale-skip/no-deployed/missing-id/missing-release/union+dedupe/3 cache-error arms); updated cadence/idle tests; model `from_backend`/`Deserialize` tests updated; service-get link branch test. All 1365 tests pass; covgate green.
- [x] **V** (2026-06-29) `cargo build -p miru-agent --features test`, `cargo clippy -p miru-agent --features test --all-targets -- -D warnings`, and the full test suite all clean. (Full `scripts/preflight.sh` deferred to the orchestrator's later step per task scope.)

Use timestamps when completing steps. Split partially-completed work into "done" / "remaining".

## Surprises & Discoveries

(Add entries as work proceeds. Seed findings from the verified context below.)

- **The worker plumbing for the uploads handle already exists** in `app/run.rs` (`init_uploads_worker` at `:259-287`, `ShutdownManager.uploads_worker_handle` field/new/shutdown step, and the `register_handle_rejects_uploads_duplicates` test). This plan only ADDS the `deployments` + `releases` stores to the already-present `init_uploads_worker` and `uploads::run` — it does NOT add a new worker or handle.
- **`backend_release.upload_rules` is the SAME source** for both the rule bodies (already written, `deployments.rs:265-277`) and the new `upload_rule_ids`. We derive ids from that one expanded array; no second fetch.
- **(2026-06-29) A SECOND backend→domain Release conversion existed beyond the syncer**: `agent/src/services/release/get.rs:20` used `models::Release::from(backend_rls)` (the fetch-release-by-id service path), not just `deployments.rs:258`. Removing the `From` impl broke its build, so it was migrated to `Release::from_backend`, deriving ids via `backend_rls.upload_rules.as_ref().map(...).unwrap_or_default()` (empty Vec when the expansion is absent — this by-id endpoint does not contractually guarantee the `upload_rules` expansion the way the syncer does, so a hard error would be wrong here). A regression test was added (`get.rs` `cache_miss_backend_release_with_upload_rules_links_ids`) to cover the new `Some(...)` branch and hold the `services/release` covgate.
- **(2026-06-29) The new non-defaulted `Release` struct field broke 4 pre-existing exhaustive `Release { .. }` literals in unrelated server tests** (`agent/tests/server/handlers.rs`, `response.rs`); each gained `upload_rule_ids: Vec::new()`. Mechanical, no behavior change.
- **(2026-06-29) `active_upload_rules` was made `pub`** (not `#[cfg(feature="test")]`-gated, since `run_impl` calls it in production) to be directly unit-testable, mirroring the existing `pub fn decide_ready` test seam. This is the cleanest way to cover all five traversal branches without driving everything through the run loop.
- **(2026-06-29) The `--lib` clippy run is NOT sufficient**: a `clippy::cloned_ref_to_slice_refs` lint in the new test code (`&[r1.clone()]` → `std::slice::from_ref(&r1)`) only surfaced under `cargo clippy --all-targets`. Always validate test code with `--all-targets`.

## Decision Log

- **Decision: "current deployment" semantics = UNION of upload rules across all `activity_status == DplActivity::Deployed` deployments (normally a singleton).** Evidence: `agent/src/deploy/apply.rs::find_target_deployed` (`:126-143`) selects deployments where `target_status == DplTarget::Deployed && error_status != DplErrStatus::Failed` and returns `ConflictingDeploymentsErr` if `len() > 1` — so the device has AT MOST ONE *target-deployed* deployment, and the deploy FSM drives that one's `activity_status` to `Deployed`. In steady state the union is therefore exactly one deployment's release rules. We still UNION (dedupe rule ids across deployments) rather than asserting exactly one, because during a redeploy/remove transition the store can briefly hold an outgoing deployment (`activity_status == Deployed`, `target_status == Archived`, heading to `Removing`) alongside the incoming one (`activity_status == Deployed`, `target_status == Deployed`); unioning avoids prematurely abandoning files still being produced by the outgoing release. No `Deployed` deployment ⇒ empty active set ⇒ worker idles. The traversal keys off `activity_status == Deployed` (the actually-running deployment), per the task's definition, not `target_status`.
  Date/Author: 2026-06-29 / plan author.
- **Decision: replace `From<backend_client::Release>` with `Release::from_backend(release, upload_rule_ids)`, mirroring `Deployment::from_backend(deployment, config_instance_ids)`.** Rationale: `Deployment` carries `config_instance_ids` via an explicit constructor param extracted by the caller (`pull_deployments` builds `cfg_inst_ids` from `backend_dpl.config_instances` and passes it to `from_backend` — `deployments.rs:91-99,219,239-265`); `Deployment` has NO `From` impl. To mirror that exactly, `Release` gets a `from_backend` constructor taking an explicit `Vec<UploadRuleID>`, and the sole existing call site (`deployments.rs:258`, the only backend→domain Release conversion in the codebase — verified by grep) switches from `.into()` to `from_backend`. Removing the `From` keeps the model at 100% covgate with no untested conversion path. (Alternative considered: derive ids inside `From` from `release.upload_rules` — rejected because it does NOT mirror the `config_instance_ids` explicit-param flow the task requires.)
  Date/Author: 2026-06-29 / plan author.
- **Decision: NO pruning of `storage::UploadRules` (append-only stays).** Explicit user decision. The store remains the by-id BODY lookup; the worker simply stops treating `.values()` as the active set and resolves the active set by traversal. Rationale: deletion races (a rule body removed while a transition still references it), and the store doubles as a durable BODY cache the traversal reads by id. Bounded growth is acceptable; bodies are small and `write_if_absent` already skips re-writes.
  Date/Author: 2026-06-29 / plan author.
- **Decision: the worker now DEPENDS on `storage::Deployments` + `storage::Releases` in addition to `storage::UploadRules`.** All three are `cache::FileCache<K,V>` already living in `storage::Storage` (`storage/mod.rs:83-85`) and reachable via `AppState.storage`. Threading two more `&FileCache` refs through `run`/`run_impl` and cloning two more `Arc`s in `init_uploads_worker` mirrors how `init_poller_worker`/`init_mqtt_worker` clone multiple `app_state.storage.*` / `app_state.*` pieces (`app/run.rs:228-320`).
  Date/Author: 2026-06-29 / plan author.
- **Decision (2026-06-29, implementer): `#[serde(default)]` on the `Deserialize` inner `upload_rule_ids` field.** As the plan's M1 note anticipated, this lets release JSON persisted before the field existed deserialize to an empty `Vec` rather than erroring. This intentionally diverges from `Deployment.config_instance_ids` (which has no such default) — the safer choice for the on-disk releases cache, and consistent with the empty-Vec `Default`.
- **Decision (2026-06-29, implementer): expose `active_upload_rules` as a `pub` test seam rather than driving every traversal branch through the run loop.** Mirrors the existing `pub fn decide_ready`. Direct unit tests deterministically cover deployed/none/missing-release/missing-rule/dedupe/error arms; not `#[cfg]`-gated because `run_impl` calls it in production.

## Outcomes & Retrospective

**Completed 2026-06-29.** All five milestones + validation done; the worker now acts only on the currently-`Deployed` deployment's release rules.

- **Final `Release::from_backend` signature:** `pub fn from_backend(release: backend_client::Release, upload_rule_ids: Vec<UploadRuleID>) -> Release` (`agent/src/models/release.rs`). The old `impl From<backend_client::Release>` was removed; both call sites (`sync/deployments.rs`, `services/release/get.rs`) migrated.
- **Final `uploads::run` signature:** `pub async fn run<SleepF, SleepFut, NowF>(options: &Options, deployments: &storage::Deployments, releases: &storage::Releases, upload_rules: &storage::UploadRules, sleep_fn: SleepF, now_fn: NowF, shutdown_signal: Pin<Box<...>>)` — `run_impl` mirrors it. New helper `pub async fn active_upload_rules(deployments, releases, upload_rules) -> Vec<UploadRule>`.
- **Measured covgate (all ≥ threshold):** `models` = **100** (req 100); `workers` = **87.63** (req 83.21); `services/release` = **91.17** (req 89.28); `sync` and `app` unchanged and green.
- **Validation status:** `cargo build -p miru-agent --features test` clean; `cargo clippy -p miru-agent --features test --all-targets -- -D warnings` clean; full test suite **1365 passed, 0 failed**. Full `scripts/preflight.sh` (`Preflight clean`) is deferred to the orchestrator's later delivery step per this task's scope; build + clippy --all-targets + tests are all green.
- **Scope held:** no pruning / no `storage::UploadRules` deletion; no digest/PUT/confirm/ledger/`delete_policy`/outbound HTTP (M3+ remain out of scope). Only the rule sourcing + release linkage changed.
- **Retrospective:** The plan was accurate; the only surprises were the second `From` call site in `services/release/get.rs` and the unrelated server-test `Release` literals — both mechanical. Making `active_upload_rules` a `pub` seam (vs. only run-loop assertions) was the key choice that kept the workers covgate comfortably above threshold.

## Context and Orientation

The agent is a Rust workspace rooted at `repos/agent/Cargo.toml`: `agent/` (binary crate — all logic), `libs/backend-api/` + `libs/device-api/` (OpenAPI-generated; do NOT hand-edit). Conventions: `repos/agent/AGENTS.md` (import ordering with `// standard crates` / `// internal crates` / `// external crates` group comments; `thiserror` errors; `#[cfg(feature = "test")]` gating; `scripts/test.sh` runs `RUST_LOG=off cargo test --features test`; per-module `.covgate`).

### Verified inputs (re-verify before finalizing)

- **Domain `Deployment` (the LINKAGE PATTERN to mirror)** — `agent/src/models/deployment.rs`:
  - Field declared at `:204`: `pub config_instance_ids: Vec<CfgInstID>,`.
  - `Default` at `:233`: `config_instance_ids: Vec::new(),`.
  - Constructor `pub fn from_backend(deployment: backend_client::Deployment, config_instance_ids: Vec<String>) -> Deployment` (`:239-265`) takes the ids as an explicit param and assigns `config_instance_ids` (`:263`). There is NO `From<backend_client::Deployment>` impl.
  - `Deserialize` reads `config_instance_ids: Vec<CfgInstID>` (`:314`, `:342`).
  - Caller `pull_deployments` extracts the ids and passes them: `let cfg_inst_ids = cfg_insts.iter().map(|inst| inst.id.clone()).collect();` then `store_deployment(storage.deployments, backend_dpl, cfg_inst_ids)` (`agent/src/sync/deployments.rs:91-99`), and `store_deployment` calls `models::Deployment::from_backend(backend_dpl, cfg_inst_ids)` (`:219`).
- **Domain `Release` (gets the new linkage)** — `agent/src/models/release.rs`:
  - Struct `:14-21`: `Release { id, version, git_commit_id: Option<String>, created_at, updated_at }` — NO rule linkage today.
  - `Default` `:23-33`; `From<backend_client::Release>` `:35-51` (the ONLY backend→domain Release conversion, used solely at `deployments.rs:258`); custom `Deserialize` `:53-82`.
- **Backend `Release`** — `libs/backend-api/src/models/release.rs:37`: `pub upload_rules: Option<Vec<models::BaseUploadRule>>` (expanded via `expand=release.upload_rules`). `BaseUploadRule.id: String`. `UploadRule::from(BaseUploadRule)` maps `id: rule.id` (`agent/src/models/upload_rule.rs:106-131`).
- **Acquisition** — `agent/src/sync/deployments.rs::store_expanded_release` (`:250-291`):
  - `:258` `let release: models::Release = backend_release.clone().into();` → write to `storage.releases` via `write_if_absent` (`:260-263`).
  - `:265` `let rules = backend_release.upload_rules.clone().ok_or_else(|| SyncErr::UploadRulesNotExpanded(...))?;` (missing array = hard error, mirrors `config_instances`).
  - `:270-277` for each `backend_rule` → `models::UploadRule::from` → `storage.upload_rules.write_if_absent(id, rule, |_,_| false)` (append-only — KEEP).
  - `fetch_active_deployments` requests `expansions = ["config_instances", "release.git_commit", "release.upload_rules"]` (`:123-127`), filtered to QUEUED + DEPLOYED (`:119-122`).
- **Stores** (all `cache::FileCache<K,V>`, in `storage::Storage`, `storage/mod.rs:83-85`):
  - `pub type Deployments = cache::FileCache<models::DeploymentID, models::Deployment>` (`storage/deployments.rs:6`); `pub type Releases = cache::FileCache<models::ReleaseID, models::Release>` (`storage/releases.rs:5`); `pub type UploadRules = cache::FileCache<models::UploadRuleID, models::UploadRule>` (`storage/upload_rules.rs`).
  - `AppState.storage: Arc<storage::Storage>`; `Storage` holds `pub deployments: Arc<Deployments>`, `pub releases: Arc<Releases>`, `pub upload_rules: Arc<UploadRules>`.
- **`cache::FileCache` query API** — `agent/src/cache/concurrent.rs`:
  - `pub async fn values(&self) -> Result<Vec<V>, CacheErr>` (`:491`); `pub async fn entries(&self) -> Result<Vec<CacheEntry<K,V>>, CacheErr>` (`:486`).
  - `pub async fn read_optional(&self, key: K) -> Result<Option<V>, CacheErr>` (`:597`) — use for release + rule-by-id lookups.
  - `pub async fn find_where<F>(&self, filter: F) -> Result<Vec<V>, CacheErr>` (`:601`) — use to select `Deployed` deployments: `deployments.find_where(|d| d.activity_status == DplActivity::Deployed)`.
- **Worker** — `agent/src/workers/uploads.rs`:
  - Current `pub async fn run<SleepF, SleepFut, NowF>(options, upload_rules: &storage::UploadRules, sleep_fn, now_fn, shutdown_signal)` (`:51-69`) and `run_impl(options, upload_rules, sleep_fn, now_fn)` (`:71-173`).
  - Active set read TODAY at `:93`: `let rules = match upload_rules.values().await { ... }` — THIS is what changes.
  - `decide_ready` (`:188-255`), `FileObservation`, `ReadyFile`, the placeholder `info!` sink (`:137-143`), per-rule cadence (`next_scan_at`), and pruning of `next_scan_at` for absent rule ids (`:156-157`) are ALL UNCHANGED.
- **Wiring** — `agent/src/app/run.rs`:
  - `init_uploads_worker` (`:259-287`) clones `app_state.storage.upload_rules` and spawns `uploads::run(&options, upload_rules.as_ref(), tokio::time::sleep, chrono::Utc::now, shutdown)`. `ShutdownManager.uploads_worker_handle` + shutdown step (`:494-504`) + `register_handle_rejects_uploads_duplicates` test (`:640-666`) already exist.
- **Test helpers/fixtures**:
  - `agent/tests/sync/helpers.rs`: `make_deployment_with_release_upload_rules(id, cfg_inst_args, rule_ids)` (`:91-103`) builds a backend deployment whose expanded release carries upload rules with the given ids; `make_deployment_with_release` (`:105-115`); `assert_upload_rule_stored`/`assert_release_stored` (`:176-192`). Existing test `stores_upload_rules_from_expanded_release` (`agent/tests/sync/deployments.rs:194-211`) is the one to EXTEND with a `release.upload_rule_ids` assertion.
  - `agent/tests/models/release.rs`: `from_backend()` (`:64`), `from_backend_invalid_dates()` (`:87`), `defaults()` (`:47`), `required_fields()`/`optional_fields()` (`:15`,`:28`) — must be updated for the new field (model covgate = 100%).
  - `agent/tests/workers/uploads.rs`: `mod run_loop` (`:191+`) with `spawn_rules` (`:194`) and `spawn_worker` (`:201`) helpers, the `Clock` mock (`agent/tests/mocks/clock.rs`), and `SleepController` (`agent/tests/mocks/error.rs`). These helpers MUST be extended to also create + populate `Deployments` + `Releases` stores.

### Covgate thresholds (verified)

- `agent/src/models/.covgate` = **100** (the `Release` field + `from_backend` + `Deserialize` MUST be fully covered).
- `agent/src/sync/.covgate` = **93.63** (the `store_expanded_release` change).
- `agent/src/workers/.covgate` = **83.21** (the `run_impl` traversal — `uploads.rs` aggregates here).
- `agent/src/app/.covgate` = **90.38** (the `init_uploads_worker` two-store wiring).
- `agent/src/storage/.covgate` = **94.83** (only touched if a helper is added; likely untouched).

### Validation tooling

- `scripts/preflight.sh` runs, in parallel: `scripts/lint.sh` (custom import linter, `cargo fmt`, `cargo machete` + diet unused-dep/deadcode, `rustsec` audit, `cargo clippy -D warnings`) and `scripts/covgate.sh` (`cargo test --features test` with coverage + per-module `.covgate` enforcement). Prints `Preflight clean` on success. (Memory note: the deadcode gate is part of `lint.sh`, run within preflight.)
- `scripts/test.sh` = `RUST_LOG=off cargo test --features test` (`--features test` REQUIRED — mocks gated behind it).
- `scripts/update-deps.sh` refreshes `Cargo.lock` (no new deps expected here, but run before lint).

## Plan of Work

### M1 — Add `upload_rule_ids` to domain `Release`

`agent/src/models/release.rs`:

- Add field to the struct (after `updated_at`), mirroring `Deployment.config_instance_ids`:
  ```rust
  pub upload_rule_ids: Vec<crate::models::UploadRuleID>,
  ```
  (Import note: `UploadRuleID = String` is exported at `models::UploadRuleID`; reference it as `crate::models::UploadRuleID` or add to the `// internal crates` use group, matching deployment.rs's `use crate::models::{config_instance::CfgInstID, ...}`.)
- `Default`: add `upload_rule_ids: Vec::new(),`.
- **Replace** `impl From<backend_client::Release> for Release` with a constructor mirroring `Deployment::from_backend`:
  ```rust
  impl Release {
      pub fn from_backend(
          release: backend_client::Release,
          upload_rule_ids: Vec<crate::models::UploadRuleID>,
      ) -> Release {
          Release {
              id: release.id,
              version: release.version,
              git_commit_id: release.git_commit_id,
              created_at: release.created_at.parse::<DateTime<Utc>>()
                  .unwrap_or(DateTime::<Utc>::UNIX_EPOCH),
              updated_at: release.updated_at.parse::<DateTime<Utc>>()
                  .unwrap_or(DateTime::<Utc>::UNIX_EPOCH),
              upload_rule_ids,
          }
      }
  }
  ```
- `Deserialize`: add `upload_rule_ids: Vec<crate::models::UploadRuleID>` to the inner `DeserializeRelease` struct and assign it in the constructed `Release` (mirror deployment.rs's plain `config_instance_ids: result.config_instance_ids` — a non-optional `Vec` field; for forward-compat with already-persisted release JSON that lacks the field, prefer `#[serde(default)]` on the inner field so old cache files deserialize to an empty `Vec` rather than erroring). Decide `#[serde(default)]` vs required during implementation; `#[serde(default)]` is safer for the persisted releases cache and matches the "empty Vec default" intent — document the choice in the Decision Log.

### M2 — Populate `upload_rule_ids` during acquisition

`agent/src/sync/deployments.rs::store_expanded_release` (`:250-291`):

- Extract the expanded upload rules FIRST (move the existing `:265` `rules` extraction above the release write), derive ids, build the release via `from_backend`:
  ```rust
  let Some(backend_release) = backend_dpl.release.as_deref() else {
      return Ok(());
  };

  // Upload rules ride on the expanded release; a missing array is a contract
  // violation (mirrors config_instances). Extract first so the ids can be
  // linked onto the domain Release, mirroring Deployment.config_instance_ids.
  let backend_rules = backend_release.upload_rules.clone().ok_or_else(|| {
      SyncErr::UploadRulesNotExpanded(UploadRulesNotExpandedErr {
          deployment_id: backend_dpl.id.clone(),
      })
  })?;
  let upload_rule_ids: Vec<models::UploadRuleID> =
      backend_rules.iter().map(|r| r.id.clone()).collect();

  let release = models::Release::from_backend(backend_release.clone(), upload_rule_ids);
  let release_id = release.id.clone();
  storage.releases.write_if_absent(release_id, release, |_, _| false).await?;

  // Rule BODIES: append-only by-id cache, UNCHANGED.
  for backend_rule in backend_rules {
      let rule: models::UploadRule = backend_rule.into();
      let id = rule.id.clone();
      storage.upload_rules.write_if_absent(id, rule, |_, _| false).await?;
  }

  // ... git_commit early-return block unchanged ...
  ```
  Note the reorder moves the `UploadRulesNotExpanded` check ahead of the release write; the error aborts the sync anyway, so behavior is equivalent (and the existing `upload_rules_not_expanded_error` test at `deployments.rs:481` still passes — re-verify). Update the function doc comment to mention the id linkage.

### M3 — Worker rule sourcing by traversal

`agent/src/workers/uploads.rs`:

- Extend `run` + `run_impl` signatures to take the two extra stores (place them next to `upload_rules`):
  ```rust
  pub async fn run<SleepF, SleepFut, NowF>(
      options: &Options,
      deployments: &storage::Deployments,
      releases: &storage::Releases,
      upload_rules: &storage::UploadRules,
      sleep_fn: SleepF,
      now_fn: NowF,
      mut shutdown_signal: Pin<Box<impl Future<Output = ()> + Send + 'static>>,
  ) where ...
  ```
  (and forward all three into `run_impl`). Add `use crate::models::deployment::DplActivity;` to the `// internal crates` group (or reference `crate::models::DplActivity`, which is re-exported at `models::DplActivity`).
- Replace the `:93` active-set load with a traversal helper. Add a small async fn (returns the active rule bodies, dedup by id):
  ```rust
  /// Resolves the active upload rules from the currently-deployed deployment(s):
  /// Deployed deployment -> release (by release_id) -> release.upload_rule_ids
  /// -> rule BODIES (by id) from the append-only UploadRules store. Unions across
  /// all Deployed deployments (normally exactly one; union covers redeploy
  /// transitions). Missing ids are skipped with a debug log. Cache errors are
  /// logged and treated as empty so the worker never crashes.
  async fn active_upload_rules(
      deployments: &storage::Deployments,
      releases: &storage::Releases,
      upload_rules: &storage::UploadRules,
  ) -> Vec<UploadRule> {
      let deployed = match deployments
          .find_where(|d| d.activity_status == DplActivity::Deployed)
          .await
      {
          Ok(d) => d,
          Err(e) => { error!("error reading deployments: {e:?}"); return Vec::new(); }
      };

      let mut seen: HashSet<UploadRuleID> = HashSet::new();
      let mut out: Vec<UploadRule> = Vec::new();
      for dpl in deployed {
          let release = match releases.read_optional(dpl.release_id.clone()).await {
              Ok(Some(r)) => r,
              Ok(None) => { debug!("release {} for deployed deployment {} not cached; skipping", dpl.release_id, dpl.id); continue; }
              Err(e) => { error!("error reading release {}: {e:?}", dpl.release_id); continue; }
          };
          for rule_id in &release.upload_rule_ids {
              if !seen.insert(rule_id.clone()) { continue; }
              match upload_rules.read_optional(rule_id.clone()).await {
                  Ok(Some(rule)) => out.push(rule),
                  Ok(None) => debug!("upload rule {rule_id} referenced by release {} not in store; skipping", release.id),
                  Err(e) => error!("error reading upload rule {rule_id}: {e:?}"),
              }
          }
      }
      out
  }
  ```
- In `run_impl`, replace the `:93-99` block with `let rules = active_upload_rules(deployments, releases, upload_rules).await;`. Everything downstream (`for rule in &rules`, cadence `next_scan_at`, `decide_ready` under `spawn_blocking`, the `info!` sink, the `next_scan_at` prune by `present_ids`, sleep computation) stays IDENTICAL — it already operates on a `Vec<UploadRule>`.
- Keep `min_poll_interval_secs`, `Options`, `decide_ready`, `FileObservation`, `ReadyFile`, and the dedupe unchanged.

### M4 — Wire the two stores into `init_uploads_worker`

`agent/src/app/run.rs::init_uploads_worker` (`:259-287`):

- Clone the two extra `Arc`s alongside the existing one:
  ```rust
  let deployments = app_state.storage.deployments.clone();
  let releases = app_state.storage.releases.clone();
  let upload_rules = app_state.storage.upload_rules.clone();
  ```
- Pass them into `uploads::run`:
  ```rust
  uploads::run(
      &options,
      deployments.as_ref(),
      releases.as_ref(),
      upload_rules.as_ref(),
      tokio::time::sleep,
      chrono::Utc::now,
      Box::pin(async move { let _ = shutdown_rx.recv().await; }),
  ).await;
  ```
  (Move the three `Arc`s into the spawned task.) No change to `ShutdownManager`, the handle registration, or shutdown ordering — those already exist.

### M5 — Tests

See Test Steps.

## Test Steps

Tests use `--features test` (run via `scripts/test.sh`). Test files mirror `agent/src/` under `agent/tests/`.

### TS1 — Sync linkage (`agent/tests/sync/deployments.rs`)

Extend `stores_upload_rules_from_expanded_release` (`:194-211`) — or add a sibling `populates_release_upload_rule_ids` — to assert the domain release's `upload_rule_ids`:
- Build the deployment via `make_deployment_with_release_upload_rules("dpl_1", cfg_inst_args, &["upl_rule_1", "upl_rule_2"])` (release id is `"dpl_1_rel"` per the helper), sync, then read the release from `f.release_stor` (add a `read_release` helper to `helpers.rs` mirroring `read_deployment`, or use `release_stor.read_optional("dpl_1_rel")`).
- Assert `release.upload_rule_ids == vec!["upl_rule_1", "upl_rule_2"]` (order preserved from the expanded array).
- Keep the existing body assertions (`assert_upload_rule_stored` for both ids) — bodies still cached append-only.
- Re-verify `upload_rules_not_expanded_error` (`:481`) still passes after the extraction reorder.

### TS2 — Model `Release` (`agent/tests/models/release.rs`) — covgate 100%

- Update `from_backend()` (`:64`): the model test for `From` must become a `Release::from_backend(backend_release, vec![...])` call; assert `upload_rule_ids` is set from the passed Vec (and a separate case asserting empty Vec when `[]` is passed). Update `from_backend_invalid_dates()` likewise.
- Update `defaults()` (`:47`) to assert `upload_rule_ids` defaults to an empty `Vec`.
- Update `required_fields()`/`optional_fields()` (`:15`,`:28`) and any round-trip `Deserialize` test so the new field is exercised — include a deserialize case for a persisted release JSON WITHOUT `upload_rule_ids` (asserting it defaults to empty, if `#[serde(default)]` is chosen) AND one WITH it.

### TS3 — Worker traversal (`agent/tests/workers/uploads.rs`, `mod run_loop`)

Extend the `spawn_rules`/`spawn_worker` helpers to also spawn + populate `Deployments` and `Releases` stores (mirror `spawn_rules`: `storage::Deployments::spawn(64, layout.deployments(), 1000)` and `storage::Releases::spawn(64, layout.releases(), 1000)`), and change `spawn_worker` to pass `deployments.as_ref()`, `releases.as_ref()`, `upload_rules.as_ref()` to `uploads::run`. A helper to seed a Deployed deployment + its release + rule bodies will be needed (construct `models::Deployment { activity_status: DplActivity::Deployed, release_id, .. Default }`, `models::Release { id: release_id, upload_rule_ids, .. Default }`, and `write_if_absent` the rule bodies). The EXISTING cadence/idle tests must be updated to seed a Deployed deployment + release referencing their rules (otherwise the active set is empty and they'd idle) — update `per_rule_cadence_two_rules`, `per_rule_cadence_single_rule` accordingly; `empty_rules_idles_at_min_interval` and `cache_error_is_treated_as_empty_and_idles` keep their no-active-set expectation (now achieved via no Deployed deployment / errored deployments store).

New cases:
- **TS3a — resolved via Deployed deployment**: one `Deployed` deployment → release with `upload_rule_ids = [r1]` → body for `r1` in the store whose glob matches a stable file ⇒ `r1` is scanned and the file becomes ready. (Assert via the cadence/observable behavior already used by the run-loop tests, or by adding a Deployed deployment to a cadence test and confirming the rule is scanned.)
- **TS3b — stale rule NOT acted on**: store contains BOTH `r1` (in the current release) and `r_stale` (a body left over from a prior release, NOT in any Deployed deployment's release `upload_rule_ids`). Even though `r_stale`'s glob matches a stable file, it is never scanned/reported. Assert the active set excludes `r_stale` (e.g. only `r1`'s file becomes ready / only `r1`'s cadence drives the sleep).
- **TS3c — no Deployed deployment ⇒ empty set / idle**: deployments store holds only non-`Deployed` deployments (e.g. `Queued`/`Archived`), so the active set is empty and the worker idles at `min_poll_interval_secs` (1s), regardless of how many rule bodies sit in `UploadRules`.
- **TS3d — missing id skipped**: a Deployed deployment's release lists `upload_rule_ids = [r1, r_missing]` but only `r1`'s body is in the store. The worker scans `r1`, skips `r_missing` (debug-logged), no panic.
- **TS3e — union over two Deployed deployments** (optional, documents the union decision): two `Deployed` deployments referencing releases with disjoint rule ids ⇒ both rule sets active (dedupe verified if they share an id).

Prefer asserting through the existing observable seams (cadence sleep durations via `SleepController`, file-readiness via temp files + `Clock`) rather than adding a log-capture dependency, consistent with the M2 plan's approach. If a direct assertion is cleaner, factor `active_upload_rules` to be callable in a unit test (it is an async fn over the three stores — testable by spawning the three caches and calling it directly), which gives a deterministic assertion of the resolved id set for TS3a/b/d/e.

### TS4 — app/run.rs

The existing `register_handle_rejects_uploads_duplicates` (`:640`), `shutdown_awaits_registered_uploads_worker_handle` (`:668`), and `shutdown_skips_absent_uploads_worker_handle` (`:688`) tests still apply unchanged (the handle wiring is unchanged). No new app test is required for the two-store clone, but confirm `app/.covgate` (90.38) still holds — `init_uploads_worker` is exercised by existing app-init tests; if coverage dips, add an init assertion.

## Validation and Acceptance

**Changes MUST NOT be published until `scripts/preflight.sh` reports `Preflight clean`.** Run from the repo root:

    cd /home/ben/miru/workbench4/repos/agent
    scripts/update-deps.sh        # refresh Cargo.lock (no new deps expected)
    scripts/preflight.sh          # lint + clippy -D warnings + fmt + machete + diet(deadcode) + audit + tests + covgate, in parallel
    # final line must be: Preflight clean

Plus the individual gates (all must pass clean):

    cargo build -p miru-agent
    scripts/test.sh                                   # RUST_LOG=off cargo test --features test
    cargo fmt -p miru-agent -- --check
    cargo clippy --package miru-agent --all-features -- -D warnings
    cargo machete

**Coverage gates** (`scripts/covgate.sh`, invoked by preflight): keep every touched module at/above its threshold — `models` = **100** (Release field + `from_backend` + `Deserialize` fully covered by TS2), `sync` = **93.63** (TS1), `workers` = **83.21** (TS3; cover `active_upload_rules`'s branches: deployed/none/missing-release/missing-rule/error), `app` = **90.38** (existing tests). The new traversal + linkage is fully testable.

Acceptance (human-verifiable):

1. Domain `Release` carries `upload_rule_ids: Vec<UploadRuleID>` (field + empty-Vec `Default` + `Deserialize`), populated during acquisition from the expanded backend release's `upload_rules`.
2. `store_expanded_release` builds the domain `Release` via `Release::from_backend(backend_release, upload_rule_ids)`; rule BODIES still written append-only to `storage::UploadRules` by id (no deletion).
3. The worker's active rule set is resolved by traversal (Deployed deployment → release → `upload_rule_ids` → bodies by id), NOT from `upload_rules.values()`. A stale rule body present in the store but not in the current release's `upload_rule_ids` is never acted on (TS3b). No Deployed deployment ⇒ empty set ⇒ idle (TS3c). Missing ids are skipped + debug-logged (TS3d).
4. `init_uploads_worker` threads `deployments` + `releases` + `upload_rules` into `uploads::run`; cadence/stability/`decide_ready`/log-sink/dedupe are unchanged.
5. No pruning / no store deletion anywhere.
6. `scripts/preflight.sh` prints `Preflight clean`.

## Idempotence and Recovery

- Changes are additive + a sourcing swap: one model field + constructor rename, one acquisition reorder, one worker sourcing change, one wiring change, and test updates. No generated code, no spec, no migrations.
- **Persisted releases cache compatibility**: releases written before this change lack `upload_rule_ids` in their JSON. Use `#[serde(default)]` on the `Deserialize` inner field so old entries load with an empty `Vec` (worker simply finds no rules for that release until the next sync re-writes it — but `write_if_absent` means it WON'T be re-written; acceptable since the active deployment's release is re-synced on the normal sync cycle, and a stale empty release only yields an empty active set, never a wrong one). Confirm/clamp during implementation; if stricter behavior is wanted, a one-time cache clear of `releases.json` on upgrade is an alternative (NOT planned here — out of scope).
- If `cargo build` fails after M3 with arity errors on `uploads::run`, finish all call sites: `run`→`run_impl` forwarding and `init_uploads_worker` (M4) plus the test `spawn_worker` (TS3).
- The worker holds only in-memory state; a restart re-derives everything from the caches + filesystem.
- Rollback: `git -C /home/ben/miru/workbench4/repos/agent checkout -- agent/src/models/release.rs agent/src/sync/deployments.rs agent/src/workers/uploads.rs agent/src/app/run.rs agent/tests/` restores pre-change state.

---

Change note (2026-06-29): Initial draft. The upload-discovery worker stops trusting the append-only `storage::UploadRules.values()` union and resolves the CURRENT active rule set by traversal: `Deployed` deployment(s) → release (`release_id`) → `release.upload_rule_ids` → rule bodies by id. Adds `upload_rule_ids` to the domain `Release` (mirroring `Deployment.config_instance_ids`; `From` → `from_backend`), populated in `store_expanded_release`. Threads `deployments` + `releases` stores through `uploads::run` and `init_uploads_worker`. Current-deployment semantics = UNION of `activity_status == Deployed` deployments (normally singleton; `find_target_deployed` enforces at-most-one target-deployed; union covers redeploy transitions). NO pruning — the store stays append-only as the by-id body cache.
