# Backend cache-miss fallback in agent services layer

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | Add HTTP fetchers, service traits, and cache-miss fallback logic for deployments, releases, and git_commits. Update server handlers and tests. |
| `backend/` | read-only | The three new endpoints (`GET /deployments/{id}`, `GET /releases/{id}`, `GET /git_commits/{id}`) were added in a separate openapi-repo PR. The agent's local copy at `agent/api/specs/backend/v02.yaml` already reflects the change. No backend edits are performed by this plan. |

This plan lives in `agent/plans/backlog/` because all code changes happen inside the agent repo.

## Purpose / Big Picture

On-device applications query the agent's local HTTP API for deployments, releases, and git commits by ID. Today, if the resource has been pruned from the agent's local cache, the agent returns 404 — even when the backend still has the record. After this change, on a local cache miss the agent transparently fetches the resource from the backend, re-caches it, and returns it to the caller. Only "doesn't exist locally AND doesn't exist on the backend" returns 404.

The on-device API contract is unchanged for the cache-hit path. The cache-miss path turns into a transparent backend round-trip that is invisible to the on-device app beyond a small latency increase.

User-visible behavior after this change:

- A device app that fetches `GET /deployments/<id>` for a deployment the agent has pruned locally will receive the full deployment body (200) instead of 404, provided the backend still has it.
- The same applies to releases and git commits.
- If the resource is truly gone (not in cache, not on the backend, or the agent cannot reach the backend with a valid token), the agent still returns a 404 to the device — preserving the existing "not found" contract.
- If the agent reaches the backend but receives a 5xx, network connection error, or response decoding error, that error propagates to the device application as a 500 — the plan does NOT collapse non-404 backend errors into 404. Only "the cache had no record AND the backend confirmed the resource does not exist" returns 404.

## Progress

- [ ] (YYYY-MM-DD HH:MMZ) M1 — HTTP fetchers: add `http::deployments::get`, create `http/releases.rs`, create `http/git_commits.rs`, wire into `http/mod.rs`. Commit.
- [ ] (YYYY-MM-DD HH:MMZ) M2 — Service trait + production impls: add `DeploymentFetcher`, `ReleaseFetcher`, `GitCommitFetcher` traits and production wrappers that hold `(&http::Client, &authn::TokenManager)` and call the HTTP functions with `with_retry`. Commit.
- [ ] (YYYY-MM-DD HH:MMZ) M3 — Service get fallback logic: update `services::deployment::get`, `services::release::get`, `services::git_commit::get` to take `Option<&Fetcher>` and implement the cache-miss fallback. Deployment fallback also re-caches expanded release and git_commit. Update `server/handlers.rs` to construct fetchers and pass them. Commit.
- [ ] (YYYY-MM-DD HH:MMZ) M4 — Tests: add new unit tests in `tests/services/{deployment,release,git_commit}/get.rs`; update the existing cache-hit / not-found tests to pass `None` for the backend. Commit.
- [ ] (YYYY-MM-DD HH:MMZ) M5 — Preflight: from `agent/`, run `./scripts/preflight.sh` and fix any lint/format/clippy/coverage issues. Commit any fixes. Final commit.

Split partially completed work into "done" and "remaining" subsections as needed. Use timestamps when checking items off.

## Surprises & Discoveries

(Add entries as you go.)

## Decision Log

- Decision: Use a trait-object-based fetcher interface (`DeploymentFetcher` / `ReleaseFetcher` / `GitCommitFetcher`) rather than passing a `Backend<'a>` struct that bundles `(&http::Client, &authn::TokenManager)`.
  Rationale: there is no existing test fixture for `TokenManager` in `agent/agent/tests/`. A trait-object approach lets the new tests pass an inline stub struct with no token manager at all. This matches other trait-based seams already in the codebase such as `http::ClientI`, and the production impl is a thin wrapper so the extra layer costs ~30 lines. The struct approach was rejected because it would force us to build a test token manager fixture just to exercise the fallback logic.
  Date/Author: 2026-04-08 / plan author.

- Decision: On a cache miss, if the token manager returns an error or the backend returns 404, the service returns the cache-miss error (`ServiceErr::CacheErr(CacheElementNotFound{...})`) rather than surfacing the token/HTTP error.
  Rationale: The on-device API contract for the cache-hit path is 200; the contract for "truly missing" is 404. Preserving 404 on token failure means a device querying an un-activated agent continues to see the same error shape, and the device is not forced to differentiate "transient auth failure" from "actually gone". The token error is logged at `debug!` level so it is still discoverable during investigation. Other backend errors (5xx, connection, decode) DO propagate — those represent "we tried and something unexpected broke", and a 500 response is the correct signal.
  Date/Author: 2026-04-08 / plan author.

- Decision: For deployments, fetch with `expand=config_instances&expand=release.git_commit` (matching `sync::deployments::fetch_active_deployments`) and also re-cache the expanded release and git_commit.
  Rationale: After a cache miss on a deployment, the device is very likely to walk deployment→release→git_commit next. Caching the expanded sub-resources in the same round-trip avoids two additional backend round trips on the cold path and matches the caching pattern the syncer already uses.
  Date/Author: 2026-04-08 / plan author.

- Decision: For releases and git_commits, use `write_if_absent` on cache re-population. For deployments, use `resolve_dpl(new, existing) → Deployment` + `write(id, dpl, |old, _| old.is_some_and(|e| e.is_dirty), Overwrite::Allow)`.
  Rationale: Releases and git_commits are immutable on the backend, so `write_if_absent` is the safe minimal write. Deployments can carry locally-derived state (`status`, `reported_status`, `is_dirty`, etc.) that must be preserved if a sync races with the cache-miss fallback and re-populates the entry between our `read_optional` and our `write`. Reusing `resolve_dpl` from `sync/deployments.rs` keeps the merge rule identical to what the syncer already does.
  Date/Author: 2026-04-08 / plan author.

- Decision: Tests will use inline concrete-struct stub fetcher impls (e.g. `StubDeploymentFetcher`) rather than extending `MockClient` in `tests/http/mock.rs`.
  Rationale: Inline stubs keep each test self-contained and focused on the service logic without dragging in the HTTP mock surface. The `MockClient` does already have `get_deployment_fn` + `Call::GetDeployment` scaffolding but nothing for releases or git_commits; extending it is deferred until an integration test actually needs it. This is called out in the out-of-scope section so the next engineer knows it is intentional.
  Date/Author: 2026-04-08 / plan author.

- Decision: Production fetcher converts `AuthnErr` to `SyncErr` explicitly via `.map_err`, rather than adding a new `ServiceErr::AuthnErr` variant.
  Rationale: `ServiceErr` already implements `From<SyncErr>`, and `SyncErr` already implements `From<AuthnErr>`; the explicit conversion is one line, vs adding a new variant (which would touch `errors.rs`, the `impl_error!` macro list, and add a `code`/`http_status`/`params` trait impl). The conversion keeps the change scoped to the new code.
  Date/Author: 2026-04-08 / plan author.

- Decision: Reuse `SyncErr::CfgInstsNotExpanded` for the missing-config-instances expansion error rather than inventing a new variant.
  Rationale: The `expand=config_instances` request guarantees the field is present on success; if it's absent it's the same backend contract violation that the syncer treats as an error, and the existing variant already has the right shape.
  Date/Author: 2026-04-08 / plan author.

- Decision: Duplicate the expanded-release walk inline in `services::deployment::get` rather than promoting `sync::deployments::store_expanded_release` to `pub(crate)` and reusing it.
  Rationale: The sync version returns a `SyncErr` and is tied to its own `Storage` struct; reusing it would force the service layer to depend on sync types it otherwise doesn't touch. The walk is 12 lines; duplication is cheaper than the cross-module coupling.
  Date/Author: 2026-04-08 / plan author.

- Decision: Duplicate the 5-line `resolve_dpl` merge logic inline in `services::deployment::get` rather than extracting `sync::deployments::resolve_dpl` to a shared location.
  Rationale: The function is 5 lines, and extracting it would require moving it to a new shared module (probably `models::deployment` or `services::deployment`) and updating its one existing call site in `sync/deployments.rs`. The duplication is cheaper than the cross-module reshuffling, and the duplication is local enough that future drift is unlikely. If the merge logic ever grows beyond ~10 lines, revisit and extract.
  Date/Author: 2026-04-08 / plan author.

- Decision: the cache-miss-fallthrough match arm catches only `ServiceErr::SyncErr(SyncErr::AuthnErr(_))`, not the broader `ServiceErr::SyncErr(_)`. Rationale: this preserves the explicit contract that only auth failures collapse to 404; any other SyncErr that future fetcher code might surface should propagate as a 500. Date: 2026-04-08.

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

### Repository layout

The agent repo is at `/home/ben/miru/workbench1/agent`. The branch `fix/cache-backups` is already checked out — the implementer must NOT switch branches. The working tree currently contains an unstaged update to `api/specs/backend/v02.yaml` (the openapi spec change that motivates this work); leave that file alone — it will be picked up by the first milestone commit.

### The three new backend endpoints

These endpoints already exist in `agent/api/specs/backend/v02.yaml`:

- `GET /deployments/{deployment_id}` — operationId `getDeployment`
- `GET /releases/{release_id}` — operationId `getRelease`
- `GET /git_commits/{git_commit_id}` — operationId `getGitCommit`

The agent does not use a generated backend client for requests — the generated `libs/backend-api` crate exposes only `models`. All backend HTTP calls are made manually via `crate::http::client::fetch`. No codegen is needed.

### Services layer — current state

Each service file is currently a thin wrapper around a cache read:

- `agent/agent/src/services/deployment/get.rs`:
      pub async fn get(deployments: &storage::Deployments, id: String) -> Result<models::Deployment, ServiceErr>
      pub async fn get_current(...) -> Result<models::Deployment, ServiceErr>

- `agent/agent/src/services/release/get.rs`:
      pub async fn get(releases: &storage::Releases, id: String) -> Result<models::Release, ServiceErr>
      pub async fn get_current(...) -> Result<models::Release, ServiceErr>

- `agent/agent/src/services/git_commit/get.rs`:
      pub async fn get(git_commits: &storage::GitCommits, id: String) -> Result<models::GitCommit, ServiceErr>

`get_current` is OUT OF SCOPE — it queries "what is currently active" from local state, not "look up by id". Do not touch `get_current`.

### Storage cache API

`crate::storage::{Deployments, Releases, GitCommits}` are type aliases for `cache::FileCache<K, V>`. Relevant methods:

- `read(id) -> Result<V, CacheErr>` — the cache-miss error variant is `CacheErr::CacheElementNotFound(CacheElementNotFound { msg, trace })`.
- `read_optional(&self, key: K) -> Result<Option<V>, CacheErr>` — returns `Ok(None)` on miss. Use this for clean miss detection. Note: `K` is taken by value, not by reference — call sites pass `id.clone()`.
- `write(key, value, is_dirty_fn, Overwrite)` — returns `Result<(), CacheErr>`.
- `write_if_absent(key, value, is_dirty_fn)` — returns `Result<(), CacheErr>`; no-op if the entry already exists.

### HTTP layer

`agent/agent/src/http/mod.rs` currently declares:

    pub mod client;
    pub mod config_instances;
    pub mod deployments;
    pub mod devices;
    pub mod errors;
    pub mod query;
    pub mod request;
    pub mod response;
    pub mod retry;

There is no `releases` or `git_commits` module yet — this plan adds them. `agent/agent/src/http/deployments.rs` has `list`, `list_all`, `update`, but no `get`.

The template for an existing HTTP function (copy the shape):

    pub async fn list(client: &impl ClientI, params: ListParams<'_>) -> Result<DeploymentList, HTTPErr> {
        let mut qp = QueryParams::new().paginate(params.pagination);
        // ... build qp ...
        qp = qp.expand(params.expansions);
        let url = format!("{}/deployments", client.base_url());
        let request = request::Params::get(&url).with_query(qp).with_token(params.token);
        super::client::fetch(client, request).await
    }

- `request::Params::get(&url)` returns a builder.
- `.with_token(token)` attaches auth.
- `.with_query(qp)` attaches query params.
- `QueryParams::new().expand(&["foo", "bar.baz"])` produces `expand=foo&expand=bar.baz` per `agent/agent/src/http/query.rs`.

`HTTPErr::RequestFailed` is a tuple variant: `RequestFailed(RequestFailed)`, where the inner `RequestFailed` struct has `status: reqwest::StatusCode`. Pattern-match via destructuring: `HTTPErr::RequestFailed(RequestFailed { status, .. }) if status == reqwest::StatusCode::NOT_FOUND => ...` (requires `use crate::http::errors::RequestFailed;`). The error enum is defined in `agent/agent/src/http/errors.rs`.

`http::with_retry(|| async { ... }).await` is the standard retry wrapper. See `agent/agent/src/sync/deployments.rs::fetch_active_deployments` for the canonical usage.

### Existing model conversions — REUSE, do not rewrite

- `models::Deployment::from_backend(deployment: backend_client::Deployment, config_instance_ids: Vec<String>) -> Deployment` at `agent/agent/src/models/deployment.rs`.
- `impl From<backend_client::Release> for models::Release` at `agent/agent/src/models/release.rs`.
- `impl From<backend_client::GitCommit> for models::GitCommit` at `agent/agent/src/models/git_commit.rs`.
- `impl From<backend_client::ConfigInstance> for models::ConfigInstance` at `agent/agent/src/models/config_instance.rs`.

`backend_client` is the local alias `use backend_api::models as backend_client;` used throughout `agent/agent/src/sync/deployments.rs`.

### Existing caching helpers in sync/deployments.rs — REUSE the patterns

- `store_deployment(storage, backend_dpl, cfg_inst_ids)` uses `resolve_dpl(new, existing) -> Deployment`, then `storage.write(id, dpl, |old, _| old.is_some_and(|e| e.is_dirty), Overwrite::Allow)`. `resolve_dpl` merges by taking only `target_status` and `updated_at` from the new payload and preserving the rest from the cached entry. On a clean miss (`existing == None`), it returns the new value unchanged.
- `store_expanded_release(storage, backend_dpl)` extracts the expanded release and git_commit from a backend deployment payload (when the deployment was fetched with `expand=release.git_commit`) and writes them via `write_if_absent`. Releases and git_commits are immutable on the backend.

The new code lives in `services/`, not `sync/`, but the approach is identical. Per the Decision Log, both `resolve_dpl` and the expanded-release walk are duplicated inline in `services::deployment::get` rather than extracted — neither sync helper is promoted to a shared location.

### Server state and handlers

`agent/agent/src/server/state.rs`:

    pub struct State {
        pub storage: Arc<Storage>,
        pub http_client: Arc<http::Client>,
        pub syncer: Arc<sync::Syncer>,
        pub token_mngr: Arc<authn::TokenManager>,
        pub activity_tracker: Arc<activity::Tracker>,
        pub event_hub: events::EventHub,
    }

`agent/agent/src/server/handlers.rs` currently has:

    pub async fn get_deployment(AxumState(state): AxumState<Arc<State>>, Path(deployment_id): Path<String>) -> impl IntoResponse {
        handle(async {
            let dpl = dpl_svc::get(&state.storage.deployments, deployment_id).await?;
            Ok::<_, ServerErr>(device_server::Deployment::from(&dpl))
        }, "Error getting deployment").await
    }

And analogous `get_release`, `get_git_commit`. These are the only production call sites of `dpl_svc::get`, `rls_svc::get`, and `git_cmt_svc::get` — there are no other in-tree callers. The only related cross-service call is `rls_svc::get_current` calling `dpl_svc::get_current`, which is OUT OF SCOPE.

### TokenManager

`crate::authn::TokenManager` is used in `agent/agent/src/sync/syncer.rs::sync_impl`:

    let token = self.token_mngr.get_token().await?;
    // ... use token.token (the &str) ...

Use the same call shape in the production fetcher wrapper. The token call CAN fail (for example before device activation). On token failure during a cache-miss fallback, the service must return the original cache-miss error to the caller (see "Behavior on errors" below).

### ServiceErr

`agent/agent/src/services/errors.rs` already has both variants needed:

- `ServiceErr::CacheErr(cache::CacheErr)` with a `From<CacheErr>` impl.
- `ServiceErr::HTTPErr(http::HTTPErr)` with a `From<HTTPErr>` impl.
- `ServiceErr::SyncErr(sync::SyncErr)` — this is how token errors surface in this plan. `TokenManager::get_token` returns `Result<Arc<Token>, authn::AuthnErr>`. `SyncErr` has `From<AuthnErr>`, so the production fetcher converts via `let token = self.token_mngr.get_token().await.map_err(|e| ServiceErr::SyncErr(sync::SyncErr::from(e)))?;`. Token failures therefore surface as `ServiceErr::SyncErr(SyncErr::AuthnErr(_))` and the M3 fallback's `Err(ServiceErr::SyncErr(sync::SyncErr::AuthnErr(_)))` arm catches them (and only them — any other `SyncErr` variant propagates as a 500). No new `ServiceErr` variants are required.

### Test infrastructure

- `agent/agent/tests/services/{deployment,release,git_commit}/get.rs` already exist. Existing setup uses:
      Deployments::spawn(16, dir.file("deployments.json"), 1000)
      Releases::spawn(16, dir.file("releases.json"), 1000)
      GitCommits::spawn(16, dir.file("git_commits.json"), 1000)
- `agent/agent/tests/http/mock.rs` has a `MockClient` with:
  - `get_deployment_fn` setter and `Call::GetDeployment` route variant ALREADY scaffolded.
  - Route match: `(GET, "/deployments/{id}") -> Call::GetDeployment`.
  - Routing is order-sensitive; if new variants are added, watch for prefix collisions.
  - NO existing `get_release_fn` / `get_git_commit_fn` / `Call::GetRelease` / `Call::GetGitCommit`.
- No `TokenManager` test fixture exists anywhere under `agent/agent/tests/`.

Because of the missing `TokenManager` fixture, the service signature uses `Option<&impl Fetcher>` so tests can supply either `None` or an inline stub that bypasses the token manager entirely. See the Decision Log.

### Preflight

`agent/scripts/preflight.sh` runs:

    "$REPO_ROOT/scripts/lint.sh"
    "$REPO_ROOT/scripts/covgate.sh"

Lint covers fmt + clippy + import-linter. `covgate` runs `scripts/test.sh` (which sets `RUST_LOG=off cargo test --features test -- --test-threads=1`) and enforces per-module coverage thresholds via `.covgate` files. New `.covgate` files are NOT required for the new HTTP modules. `.covgate` files live at module-directory level only, and the existing `agent/agent/src/http/.covgate` (containing `92.95`) covers the entire `http/` module — including the new `releases.rs` and `git_commits.rs`. The only risk is that adding new untested code dips the aggregate coverage below 92.95%; if `./scripts/covgate.sh` fails on the http module, add targeted unit tests for the new `get` functions until coverage rises back above the threshold.

## Plan of Work

This section walks through the edits in the order they should be applied. Each milestone ends in a commit. Everything is inside `agent/agent/src/` and `agent/agent/tests/`; the implementer does not leave the agent repo.

### M1 — HTTP fetchers

Files touched:

- `agent/agent/src/http/deployments.rs` — add:

      pub async fn get(
          client: &impl ClientI,
          id: &str,
          expansions: &[&str],
          token: &str,
      ) -> Result<backend_client::Deployment, HTTPErr> {
          let qp = QueryParams::new().expand(expansions);
          let url = format!("{}/deployments/{}", client.base_url(), id);
          let request = request::Params::get(&url).with_query(qp).with_token(token);
          super::client::fetch(client, request).await
      }

  Adjust the exact backend model path (`backend_client::Deployment` vs `backend_api::models::Deployment`) to match the convention used elsewhere in the file. Preserve existing imports; add the alias if the file does not already have one.

- `agent/agent/src/http/releases.rs` — NEW FILE. Mirrors `http/deployments.rs::get` but for releases. No expansions needed initially — callers can pass `&[]`. Return `Result<backend_client::Release, HTTPErr>`. URL is `format!("{}/releases/{}", client.base_url(), id)`.

- `agent/agent/src/http/git_commits.rs` — NEW FILE. Same pattern for git_commits. Return `Result<backend_client::GitCommit, HTTPErr>`. URL is `format!("{}/git_commits/{}", client.base_url(), id)`.

- `agent/agent/src/http/mod.rs` — add `pub mod releases;` and `pub mod git_commits;`. Keep alphabetical ordering.

Commit message for M1 (hardcoded — do not change):

    feat(http): add GET endpoints for deployments, releases, git_commits

### M2 — Service trait + production impls

Files touched / created:

- `agent/agent/src/services/deployment/get.rs` — add:

      use crate::http;
      use crate::authn;
      use backend_api::models as backend_client;

      #[allow(async_fn_in_trait)]
      pub trait DeploymentFetcher {
          async fn fetch_deployment(&self, id: &str) -> Result<backend_client::Deployment, ServiceErr>;
      }

      pub struct HttpDeploymentFetcher<'a> {
          pub client: &'a http::Client,
          pub token_mngr: &'a authn::TokenManager,
      }

      impl<'a> DeploymentFetcher for HttpDeploymentFetcher<'a> {
          async fn fetch_deployment(&self, id: &str) -> Result<backend_client::Deployment, ServiceErr> {
              // TokenManager::get_token returns AuthnErr; we convert via SyncErr because
              // ServiceErr already implements From<SyncErr> and SyncErr already implements
              // From<AuthnErr>. This avoids adding a new error variant.
              let token = self
                  .token_mngr
                  .get_token()
                  .await
                  .map_err(|e| ServiceErr::SyncErr(sync::SyncErr::from(e)))?;
              http::with_retry(|| async {
                  http::deployments::get(
                      self.client,
                      id,
                      &["config_instances", "release.git_commit"],
                      &token.token,
                  )
                  .await
              })
              .await
              .map_err(ServiceErr::from)
          }
      }

  Adjust `with_retry`'s exact call shape to match whatever `sync::deployments::fetch_active_deployments` does — copy the pattern.

- `agent/agent/src/services/release/get.rs` — same pattern. `ReleaseFetcher::fetch_release(&self, id: &str) -> Result<backend_client::Release, ServiceErr>`. `HttpReleaseFetcher` calls `http::releases::get`. No expansions.

- `agent/agent/src/services/git_commit/get.rs` — same pattern. `GitCommitFetcher::fetch_git_commit(&self, id: &str) -> Result<backend_client::GitCommit, ServiceErr>`. `HttpGitCommitFetcher` calls `http::git_commits::get`.

Commit message for M2 (hardcoded — do not change):

    feat(services): add backend fetcher traits for cache-miss fallback

### M3 — Service get fallback logic

Update each service's `get` function. New signature for deployment:

    pub async fn get<F: DeploymentFetcher>(
        deployments: &storage::Deployments,
        releases: &storage::Releases,
        git_commits: &storage::GitCommits,
        backend: Option<&F>,
        id: String,
    ) -> Result<models::Deployment, ServiceErr>

Deployments need access to `releases` and `git_commits` too because of expanded sub-resource caching. Releases and git_commits only need their own cache.

For release:

    pub async fn get<F: ReleaseFetcher>(
        releases: &storage::Releases,
        backend: Option<&F>,
        id: String,
    ) -> Result<models::Release, ServiceErr>

For git_commit:

    pub async fn get<F: GitCommitFetcher>(
        git_commits: &storage::GitCommits,
        backend: Option<&F>,
        id: String,
    ) -> Result<models::GitCommit, ServiceErr>

Implementation per the "Behavior on errors" flow below. Define a local private `resolve_dpl` helper at the bottom of `services/deployment/get.rs`. The body is identical to the sync version (which we cannot reuse because it is private and the Decision Log forbids extraction). The core structure is:

    let cached = deployments.read_optional(id.clone()).await?;
    if let Some(dpl) = cached {
        return Ok(dpl);
    }
    let Some(backend) = backend else {
        return Err(cache_miss_err(&id, "deployment"));
    };
    let backend_dpl = match backend.fetch_deployment(&id).await {
        Ok(d) => d,
        Err(ServiceErr::SyncErr(sync::SyncErr::AuthnErr(e))) => {
            // Token failure: AuthnErr -> SyncErr -> ServiceErr::SyncErr.
            // Falls through to cache-miss error per the Decision Log: clients
            // benefit more from "not found" than from internal auth state.
            tracing::debug!(error = ?e, id = %id, "token error during cache-miss fallback; returning NotFound");
            return Err(cache_miss_err(&id, "deployment"));
        }
        Err(ServiceErr::HTTPErr(http::HTTPErr::RequestFailed(RequestFailed { status, .. }))) if status == reqwest::StatusCode::NOT_FOUND => {
            return Err(cache_miss_err(&id, "deployment"));
        }
        Err(other) => return Err(other),
    };

    // Re-cache deployment (preserving local state if a sync raced us).
    // backend_dpl.config_instances is Option<Vec<ConfigInstance>>; when we
    // request expand=config_instances the backend must populate it. If it
    // doesn't, that's the same contract violation the syncer reports via
    // SyncErr::CfgInstsNotExpanded — reuse that variant.
    let cfg_insts = backend_dpl.config_instances.clone().ok_or_else(|| {
        ServiceErr::SyncErr(sync::SyncErr::CfgInstsNotExpanded(
            sync::errors::CfgInstsNotExpandedErr {
                deployment_id: backend_dpl.id.clone(),
            },
        ))
    })?;
    let cfg_inst_ids: Vec<String> = cfg_insts.iter().map(|ci| ci.id.clone()).collect();
    let new_dpl = models::Deployment::from_backend(backend_dpl.clone(), cfg_inst_ids);
    let existing = deployments.read_optional(id.clone()).await.ok().flatten();
    let merged = resolve_dpl(new_dpl, existing);
    if let Err(e) = deployments.write(
        id.clone(),
        merged.clone(),
        |old, _| old.is_some_and(|e| e.is_dirty),
        Overwrite::Allow,
    ).await {
        tracing::error!(error = ?e, id = %id, "failed to cache fetched deployment; returning value anyway");
    }

    // Cache the expanded release if present.
    if let Some(backend_release) = backend_dpl.release.as_deref() {
        let release: models::Release = backend_release.clone().into();
        let release_id = release.id.clone();
        if let Err(e) = releases.write_if_absent(release_id, release, |_, _| false).await {
            tracing::error!("failed to cache expanded release on cache-miss: {e}");
        }
        // Cache the expanded git_commit if present.
        if let Some(Some(backend_gc)) = &backend_release.git_commit {
            let gc: models::GitCommit = (*backend_gc.clone()).into();
            let gc_id = gc.id.clone();
            if let Err(e) = git_commits.write_if_absent(gc_id, gc, |_, _| false).await {
                tracing::error!("failed to cache expanded git_commit on cache-miss: {e}");
            }
        }
    }

    Ok(merged)

And at the bottom of `services/deployment/get.rs`, add the local helper:

    // Inlined from sync::deployments::resolve_dpl per the Decision Log:
    // we duplicate this 5-line merge instead of promoting the sync function
    // to pub(crate), to keep the service layer free of sync dependencies.
    fn resolve_dpl(new: models::Deployment, cached: Option<models::Deployment>) -> models::Deployment {
        match cached {
            Some(cached) => models::Deployment {
                target_status: new.target_status,
                updated_at: new.updated_at,
                ..cached
            },
            None => new,
        }
    }

This walk is the canonical pattern from `sync::deployments::store_expanded_release`: `backend_dpl.release` is `Option<Box<Release>>` (hence `.as_deref()`), and `backend_release.git_commit` is `Option<Option<Box<GitCommit>>>` (hence the `Some(Some(...))` binding). See the Decision Log for why we duplicate this walk inline rather than reusing the sync helper.

`resolve_dpl` currently lives in `agent/agent/src/sync/deployments.rs`. The cache-miss handler in `services::deployment::get` will duplicate the 5-line `resolve_dpl` merge logic inline (not extract `sync::deployments::resolve_dpl` to a shared location). Rationale: the function is 5 lines, and extracting it would require moving it to a new shared module (probably `models::deployment` or `services::deployment`) and updating its one existing call site in `sync/deployments.rs`. The duplication is cheaper than the cross-module reshuffling, and the duplication is local enough that future drift is unlikely. If the merge logic ever grows beyond ~10 lines, revisit and extract. The decision is also recorded in the Decision Log.

Define a private `cache_miss_err(id: &str, kind: &str) -> ServiceErr` helper at the bottom of each get.rs file. Do not extract to a shared module — keeping per-file duplicates avoids touching `services/mod.rs` or `services/errors.rs` and keeps the M3 commit's file list closed. Each of `services/{deployment,release,git_commit}/get.rs` defines its own copy:

    use crate::cache::errors::{CacheErr, CacheElementNotFound};

    fn cache_miss_err(id: &str, kind: &str) -> ServiceErr {
        ServiceErr::CacheErr(CacheErr::CacheElementNotFound(CacheElementNotFound {
            msg: format!("{kind} '{id}' not found in cache"),
            trace: crate::trace!(),
        }))
    }

The `crate::trace!()` macro is defined in `agent/agent/src/errors/mod.rs:52` and returns `Box<Trace>`, matching the type `CacheElementNotFound.trace` expects.

For releases and git_commits, the flow is the same minus the expanded-subresource caching:

    let cached = releases.read_optional(id.clone()).await?;
    if let Some(rls) = cached {
        return Ok(rls);
    }
    let Some(backend) = backend else {
        return Err(cache_miss_err(&id, "release"));
    };
    let backend_rls = match backend.fetch_release(&id).await {
        Ok(r) => r,
        Err(ServiceErr::SyncErr(sync::SyncErr::AuthnErr(e))) => {
            // Token failure: AuthnErr -> SyncErr -> ServiceErr::SyncErr.
            // Falls through to cache-miss error per the Decision Log: clients
            // benefit more from "not found" than from internal auth state.
            tracing::debug!(error = ?e, id = %id, "token error during cache-miss fallback; returning NotFound");
            return Err(cache_miss_err(&id, "release"));
        }
        Err(ServiceErr::HTTPErr(http::HTTPErr::RequestFailed(RequestFailed { status, .. }))) if status == reqwest::StatusCode::NOT_FOUND => {
            return Err(cache_miss_err(&id, "release"));
        }
        Err(other) => return Err(other),
    };
    let rls_model = models::Release::from(backend_rls);
    if let Err(e) = releases.write_if_absent(id.clone(), rls_model.clone(), |_, _| false).await {
        tracing::error!(error = ?e, id = %id, "failed to cache fetched release; returning value anyway");
    }
    Ok(rls_model)

git_commits is identical with `GitCommit` substituted.

### Behavior on errors (write this into the code as the exact policy)

1. Cache hit (`read_optional` -> `Ok(Some(v))`): return v. Backend not called, token not fetched.
2. Cache miss (`read_optional` -> `Ok(None)`):
   a. If `backend` is `None`: return `ServiceErr::CacheErr(CacheElementNotFound{...})` via `cache_miss_err`.
   b. If token fetch fails: `debug!`-log the token error, return the cache-miss error. Rationale in the Decision Log.
   c. If backend fetch returns 404 (`HTTPErr::RequestFailed(RequestFailed { status, .. })` with `status == reqwest::StatusCode::NOT_FOUND`): return the cache-miss error.
   d. If backend fetch returns any other HTTP error (5xx, network, decode): propagate as `ServiceErr::HTTPErr`. The handler maps it through the existing `ServerErr` conversion. Do not silently swallow.
   e. If backend fetch succeeds: cache the value (plus expanded sub-resources for deployments) and return it. A cache-write failure is `error!`-logged but the read still succeeds — degrading the read because of a cache-write failure would be wrong.
3. Cache `read_optional` returning `Err(...)` (not a miss, a broken cache): propagate. Do not reach the backend.

### Handler updates

`agent/agent/src/server/handlers.rs` — update `get_deployment`, `get_release`, `get_git_commit` to construct fetchers and pass them through:

    pub async fn get_deployment(AxumState(state): AxumState<Arc<State>>, Path(deployment_id): Path<String>) -> impl IntoResponse {
        handle(async {
            let fetcher = dpl_svc::HttpDeploymentFetcher {
                client: state.http_client.as_ref(),
                token_mngr: state.token_mngr.as_ref(),
            };
            let dpl = dpl_svc::get(
                &state.storage.deployments,
                &state.storage.releases,
                &state.storage.git_commits,
                Some(&fetcher),
                deployment_id,
            ).await?;
            Ok::<_, ServerErr>(device_server::Deployment::from(&dpl))
        }, "Error getting deployment").await
    }

Analogous changes for `get_release` and `get_git_commit`. These handlers are the only production call sites — there is no risk of missing a caller.

Commit message for M3 (hardcoded — do not change):

    feat(services): fall back to backend on cache-miss for deployments, releases, git_commits

### M4 — Tests

All tests live under `agent/agent/tests/services/{deployment,release,git_commit}/get.rs`. The existing tests must continue to pass; their signatures WILL change because the service signature changes. This is expected.

For each of the three resources, the implementer adds (at minimum) these new tests:

- `cache_hit_no_backend_call` — pre-populate the cache; call the service with a stub fetcher whose `fetch_*` method panics if called; assert the returned value matches; assert panic did not occur (implicit — the test would fail).
- `cache_miss_backend_hit_caches_value` — empty cache; stub fetcher returns a synthetic backend payload; assert returned value matches. Then call the service AGAIN with a panicking stub fetcher and assert it returns the cached value — this proves the first call re-cached.
- `cache_miss_backend_404_returns_not_found` — empty cache; stub returns `HTTPErr::RequestFailed(RequestFailed { status: NOT_FOUND, .. })` wrapped in `ServiceErr::HTTPErr`; assert `ServiceErr::CacheErr(CacheElementNotFound)`.
- `cache_miss_backend_500_returns_error` — empty cache; stub returns `HTTPErr::RequestFailed(RequestFailed { status: INTERNAL_SERVER_ERROR, .. })`; assert `ServiceErr::HTTPErr(...)`.
- `cache_miss_backend_network_err_returns_error` — empty cache; stub returns `HTTPErr::ReqwestErr { kind: Connection, .. }` or `HTTPErr::MockErr { is_network_conn_err: true }` — whichever variant the existing HTTPErr enum supports. Assert `ServiceErr::HTTPErr(...)`.
- `cache_miss_token_err_returns_not_found` — stub returns `ServiceErr::SyncErr(sync::SyncErr::AuthnErr(...))` simulating a token (auth) failure. Assert `ServiceErr::CacheErr(CacheElementNotFound)`. Note: only `AuthnErr` is swallowed — any non-`AuthnErr` `SyncErr` from the fetcher would propagate and surface as a 500.
- `cache_miss_no_backend_returns_not_found` — call with `None::<&SomeStub>` as the backend. Assert `ServiceErr::CacheErr(CacheElementNotFound)`.

Plus, ONE deployment-specific test:

- `cache_miss_backend_hit_caches_expanded_release_and_git_commit` — empty caches; stub fetcher returns a backend deployment that has an expanded release containing an expanded git_commit; call the service; assert the returned deployment is correct; then assert `releases.read_optional(release_id).await.unwrap().is_some()` and `git_commits.read_optional(git_commit_id).await.unwrap().is_some()`.

The inline stub for the tests is a concrete struct that holds a canned `Result` and a call counter. Using a concrete struct (rather than a closure-backed generic) avoids the `async_fn_in_trait` + `Fn` + `Send` bound-juggling issues, and the counter supports "verify stub was called exactly N times" assertions directly:

    use std::sync::Mutex;

    struct StubDeploymentFetcher {
        result: Mutex<Option<Result<backend_client::Deployment, ServiceErr>>>,
        call_count: std::sync::atomic::AtomicUsize,
    }

    impl StubDeploymentFetcher {
        fn ok(dpl: backend_client::Deployment) -> Self {
            Self {
                result: Mutex::new(Some(Ok(dpl))),
                call_count: Default::default(),
            }
        }
        fn err(e: ServiceErr) -> Self {
            Self {
                result: Mutex::new(Some(Err(e))),
                call_count: Default::default(),
            }
        }
        fn calls(&self) -> usize {
            self.call_count.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    impl DeploymentFetcher for StubDeploymentFetcher {
        async fn fetch_deployment(&self, _id: &str) -> Result<backend_client::Deployment, ServiceErr> {
            self.call_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.result
                .lock()
                .unwrap()
                .take()
                .expect("stub called more times than canned results provided")
        }
    }

For the "panic if called" case (cache-hit tests that must NOT invoke the backend), do NOT use `StubDeploymentFetcher`. Either pass `None::<&StubDeploymentFetcher>` as the `backend` argument, or define a tiny inline panicking stub:

    struct PanicFetcher;
    impl DeploymentFetcher for PanicFetcher {
        async fn fetch_deployment(&self, _: &str) -> Result<backend_client::Deployment, ServiceErr> {
            panic!("backend should not be called on cache hit")
        }
    }

Releases and git_commits use the same shape with their respective backend types (`StubReleaseFetcher` → `backend_client::Release`, `StubGitCommitFetcher` → `backend_client::GitCommit`).

The existing cache-hit / not-found tests each need one-line updates: add the extra cache args (for deployment) and pass `None::<&StubDeploymentFetcher>` (or the release/git_commit equivalent) or a never-called `PanicFetcher`.

Commit message for M4 (hardcoded — do not change):

    test(services): cover cache-miss backend fallback paths

### M5 — Preflight

From `agent/`:

    ./scripts/preflight.sh

Expect `clean`. If it reports `capped` or any failure, fix the lint/format/clippy/coverage issue identified in the output and re-run. Typical issues the implementer should expect:

- `cargo fmt` rewrites on new files — re-run and commit the fmt result into the same M5 commit.
- `clippy::needless_borrow` or `clippy::needless_lifetimes` on the new trait impls — fix in place.
- `.covgate` thresholds for touched modules — if coverage dips, add one or two targeted tests to lift it above the threshold.
- New `.covgate` files are NOT required. The existing `agent/agent/src/http/.covgate` (containing `92.95`) covers the entire `http/` module, including the new `releases.rs` and `git_commits.rs`. The risk is only that adding new untested code dips the aggregate coverage below 92.95% — if `./scripts/covgate.sh` fails on the http module, add targeted unit tests for the new `get` functions until coverage rises back above the threshold.

Commit message for M5 (hardcoded — do not change; only if there are fixes to commit):

    chore(preflight): satisfy lint and coverage gates

If preflight is clean on the first run, there is nothing to commit for M5 — move straight to opening the PR.

## Concrete Steps

Every command below states its working directory explicitly. All paths are absolute.

### Pre-work: verify branch and working tree

    cd /home/ben/miru/workbench1/agent
    git status
    git branch --show-current

Expected:

- Branch is `fix/cache-backups`. DO NOT switch branches.
- `api/specs/backend/v02.yaml` shows as modified (unstaged). Leave it alone — it will be picked up by the first commit.

### M1 — HTTP fetchers

    cd /home/ben/miru/workbench1/agent/agent

1. Edit `src/http/deployments.rs` and append a `pub async fn get(...)` per "Plan of Work / M1".
2. Create `src/http/releases.rs` with `pub async fn get(...)`.
3. Create `src/http/git_commits.rs` with `pub async fn get(...)`.
4. Edit `src/http/mod.rs` and add `pub mod releases;` and `pub mod git_commits;` in alphabetical order.
5. Build to verify:

        cd /home/ben/miru/workbench1/agent/agent
        cargo build --features test

   Expected: clean build with no warnings on the touched files. Fix anything the compiler flags before proceeding.

6. Commit from the agent repo root:

        cd /home/ben/miru/workbench1/agent
        git add api/specs/backend/v02.yaml agent/src/http/deployments.rs agent/src/http/releases.rs agent/src/http/git_commits.rs agent/src/http/mod.rs
        git commit -m "feat(http): add GET endpoints for deployments, releases, git_commits"

   NOTE: the first commit of this change is the one that pulls in `api/specs/backend/v02.yaml`. Subsequent milestones do not re-stage it.

### M2 — Service fetcher traits + production impls

    cd /home/ben/miru/workbench1/agent/agent

1. Edit `src/services/deployment/get.rs`: add `DeploymentFetcher` trait and `HttpDeploymentFetcher` impl.
2. Edit `src/services/release/get.rs`: add `ReleaseFetcher` + `HttpReleaseFetcher`.
3. Edit `src/services/git_commit/get.rs`: add `GitCommitFetcher` + `HttpGitCommitFetcher`.
4. Build:

        cargo build --features test

5. Commit:

        cd /home/ben/miru/workbench1/agent
        git add agent/src/services/deployment/get.rs agent/src/services/release/get.rs agent/src/services/git_commit/get.rs
        git commit -m "feat(services): add backend fetcher traits for cache-miss fallback"

### M3 — Service get fallback logic + handler updates

    cd /home/ben/miru/workbench1/agent/agent

1. Update each `services/*/get.rs` `get` function per "Plan of Work / M3". New signatures, the `read_optional` early-return flow, the 404/token-err fallthrough, and the cache re-population.
2. Duplicate the 5-line `resolve_dpl` merge logic inline in `services::deployment::get` per the Decision Log (do NOT extract from `sync/deployments.rs`).
3. Add the `cache_miss_err(id, kind)` helper as a private free function at the bottom of each of `services/{deployment,release,git_commit}/get.rs`. Do not extract to a shared module — keeping per-file duplicates avoids touching `services/mod.rs` or `services/errors.rs` and keeps the M3 commit's file list closed.
4. Update `src/server/handlers.rs`:
   - `get_deployment` constructs `HttpDeploymentFetcher` and passes `Some(&fetcher)` plus the extra `releases` and `git_commits` caches.
   - `get_release` constructs `HttpReleaseFetcher` and passes `Some(&fetcher)`.
   - `get_git_commit` constructs `HttpGitCommitFetcher` and passes `Some(&fetcher)`.
5. Build:

        cargo build --features test

6. Run tests to confirm nothing else breaks:

        ./scripts/test.sh

   Expected: existing tests that call `dpl_svc::get(&storage.deployments, id)` (old signature) FAIL to compile. Those will be fixed in M4. If they currently fail here, that is EXPECTED — proceed to commit and do M4 next.

   If the tests module does not compile, and only the tests module does not compile, it is safe to commit. If `src/` does not compile, fix it before committing.

7. Commit:

        cd /home/ben/miru/workbench1/agent
        git add agent/src/services/deployment/get.rs agent/src/services/release/get.rs agent/src/services/git_commit/get.rs agent/src/server/handlers.rs
        git commit -m "feat(services): fall back to backend on cache-miss for deployments, releases, git_commits"

### M4 — Tests

    cd /home/ben/miru/workbench1/agent/agent

1. Edit `tests/services/deployment/get.rs`:
   - Define the inline `StubDeploymentFetcher` (or equivalent).
   - Update existing tests to use the new signature (pass `&storage.releases`, `&storage.git_commits`, `None` for backend, unchanged id).
   - Add all the new `cache_miss_*`, `cache_hit_no_backend_call`, and `cache_miss_backend_hit_caches_expanded_release_and_git_commit` tests.
2. Edit `tests/services/release/get.rs`: same pattern, no expanded test.
3. Edit `tests/services/git_commit/get.rs`: same pattern, no expanded test.
4. Run tests:

        ./scripts/test.sh

   Expected: all existing tests pass (because their signatures are updated) and all new tests pass. If a test fails, read its output carefully — especially the `cache_miss_token_err_returns_not_found` test, which must produce a `ServiceErr::CacheErr(CacheElementNotFound)` and NOT a `ServiceErr::SyncErr`.

5. Commit:

        cd /home/ben/miru/workbench1/agent
        git add agent/tests/services/deployment/get.rs agent/tests/services/release/get.rs agent/tests/services/git_commit/get.rs
        git commit -m "test(services): cover cache-miss backend fallback paths"

### M5 — Preflight

    cd /home/ben/miru/workbench1/agent
    ./scripts/preflight.sh

Expected output ends with `clean`. If it reports `capped` or any failure:

- Read the failure (format / clippy / import-linter / covgate).
- Fix in place.
- Re-run preflight.
- If fixes are necessary, commit them at the end of M5:

        cd /home/ben/miru/workbench1/agent
        git add <touched files>
        git commit -m "chore(preflight): satisfy lint and coverage gates"

If preflight is clean on first run, no M5 commit is needed.

## Validation and Acceptance

### Preflight gate (VERBATIM)

preflight (./scripts/preflight.sh or the agent's preflight checks invoked through $preflight) must report `clean` before a PR is opened. This is a hard gate enforced by the orchestrator. If preflight reports `capped` or any failures, the orchestrator must NOT push or open a PR.

### Per-test acceptance criteria

All of the following must pass after M4:

- `cache_hit_no_backend_call` (deployment / release / git_commit): a pre-populated cache entry is returned without the stub fetcher's `fetch_*` being invoked. Use either `None::<&StubDeploymentFetcher>` (etc.) or a `PanicFetcher` whose `fetch_*` method calls `panic!("backend should not be called on cache hit")`.
- `cache_miss_backend_hit_caches_value` (deployment / release / git_commit): empty cache, stub returns a synthetic payload, service returns the corresponding `models::*` value. A second call with a panicking stub returns the same value without invoking the stub — proving re-cache.
- `cache_miss_backend_404_returns_not_found` (deployment / release / git_commit): empty cache, stub returns `HTTPErr::RequestFailed(RequestFailed { status: NOT_FOUND, .. })`. Service returns `ServiceErr::CacheErr(CacheElementNotFound(...))`.
- `cache_miss_backend_500_returns_error` (deployment / release / git_commit): empty cache, stub returns `HTTPErr::RequestFailed(RequestFailed { status: INTERNAL_SERVER_ERROR, .. })`. Service returns `ServiceErr::HTTPErr(HTTPErr::RequestFailed(..))`.
- `cache_miss_backend_network_err_returns_error` (deployment / release / git_commit): empty cache, stub returns a connection error variant of `HTTPErr`. Service returns `ServiceErr::HTTPErr(...)`.
- `cache_miss_token_err_returns_not_found` (deployment / release / git_commit): empty cache, stub returns `ServiceErr::SyncErr(sync::SyncErr::AuthnErr(...))` to simulate an auth failure. Service returns `ServiceErr::CacheErr(CacheElementNotFound(...))`. Any other `SyncErr` variant (non-`AuthnErr`) from the fetcher would propagate and surface as a 500.
- `cache_miss_no_backend_returns_not_found` (deployment / release / git_commit): empty cache, backend is `None`. Service returns `ServiceErr::CacheErr(CacheElementNotFound(...))`.
- `cache_miss_backend_hit_caches_expanded_release_and_git_commit` (deployment only): empty caches, stub returns a deployment with expanded release and git_commit. After the call, `releases.read_optional(release_id).await.unwrap().is_some()` and `git_commits.read_optional(git_commit_id).await.unwrap().is_some()`.

All existing tests in `tests/services/{deployment,release,git_commit}/get.rs` must continue to pass with their updated signatures.

### Observable end-to-end behavior

With the agent running and locally pruned of a deployment/release/git_commit that the backend still has:

- `curl http://localhost:<agent_port>/deployments/<id>` returns 200 with the full deployment JSON. Before this change, it would have returned 404.
- `curl http://localhost:<agent_port>/releases/<id>` returns 200.
- `curl http://localhost:<agent_port>/git_commits/<id>` returns 200.
- A subsequent call to any of the three returns 200 without the agent making a backend request (verify by examining backend access logs or by watching the agent's debug logs).
- If the backend truly does not have the resource, the agent returns 404 (unchanged behavior).
- If the agent has no activation token, cache-miss still returns 404 (not an auth error), per the Decision Log.

### Test command

From `agent/`:

    ./scripts/test.sh

Expect: all existing tests pass, plus the new cache-miss fallback tests. Coverage (via `./scripts/covgate.sh`) remains at or above thresholds for the touched modules.

## Idempotence and Recovery

- **All edits are idempotent.** Adding the HTTP fetchers, traits, and fallback logic is pure code addition; re-running a milestone after a partial failure is safe because every step is a deterministic code edit.
- **Commits are per-milestone.** If M4 fails partway, the implementer can `git reset --soft HEAD~1` to un-commit the incomplete work, fix it, and re-commit. Do not use `git commit --amend` against a previous milestone's commit — that would rewrite history across milestone boundaries.
- **Preflight failures are recoverable.** If `./scripts/preflight.sh` reports a failure after M5, fix in place and re-run. If fixes require touching earlier milestone files, stage them into the M5 commit (not a separate one per file).
- **Branch safety.** The implementer must not switch branches during the run. All commits land on `fix/cache-backups`. If somehow the branch was switched, the recovery path is:

        cd /home/ben/miru/workbench1/agent
        git stash
        git checkout fix/cache-backups
        git stash pop

- **Handling a sync race while running tests.** The test stubs are synchronous and deterministic; there is no race possible within tests. In production, the `resolve_dpl` + `write(..., is_dirty_fn, Overwrite::Allow)` pattern handles the race with a concurrent syncer (same as `sync::deployments::store_deployment` does today).
- **Working-tree file to preserve.** `agent/api/specs/backend/v02.yaml` is unstaged at the start of work. It must be included in the M1 commit and not left dangling.

## Out of scope

- Regenerating `libs/backend-api`. Only `models` exist there; the agent calls the backend manually.
- The openapi spec change itself — already in the working tree from a separate openapi-repo PR.
- Modifying the syncer or any background pull behavior.
- Modifying `get_current` in any service.
- Changes to non-services-layer call paths or other resource types.
- Adding `MockClient` route handlers for releases and git_commits in `agent/agent/tests/http/mock.rs`. `get_deployment_fn` and `Call::GetDeployment` are already scaffolded; no `get_release_fn` / `get_git_commit_fn` exist. The new tests will use inline stub fetcher impls rather than `MockClient`, so extending `mock.rs` is deferred. If a future integration test needs the MockClient route, it should be added in that test's own change — not here.
