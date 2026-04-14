# Consolidate cache-miss fallback under a single BackendFetcher trait

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | All source and test changes land here. |

This plan lives in `agent/plans/backlog/` because every file touched is under `agent/agent/`.

Working directory for every command below is `agent/` (the `agent` submodule root) unless stated otherwise. The branch `refactor/services-backend-fetcher` is already checked out; it is stacked on top of `fix/cache-backups` (in-flight PR #24). Do not switch branches. The eventual PR targets `fix/cache-backups`; it will auto-retarget to `main` when PR #24 merges.

## Purpose / Big Picture

Internal refactor with no user-visible behavior change. The agent's services layer currently has three near-identical fetcher traits (`DeploymentFetcher`, `ReleaseFetcher`, `GitCommitFetcher`), three near-identical production wrappers (`HttpDeploymentFetcher`, `HttpReleaseFetcher`, `HttpGitCommitFetcher`), three near-identical test stubs, and a `cache_miss_err` helper duplicated in each of three `get.rs` files. All 1128 existing tests must still pass.

After the refactor:

- One `BackendFetcher` trait at the services layer exposes `fetch_deployment`, `fetch_release`, `fetch_git_commit`.
- One production implementation, `HttpBackend<'a, C, T>`, generic over `http::ClientI` and a new `authn::TokenManagerI` trait, unit-testable end-to-end via `MockClient` + `StubTokenManager`.
- One `StubBackend` + one `PanicBackend` in `tests/services/backend_stub.rs` replace the three per-resource stubs.
- `cache_miss_err` exists once in `services/backend.rs`.
- Net diff is negative.
- Services coverage gate ratchets back up from `93.59` once new `HttpBackend` integration tests land.

## Progress

- [ ] M1 TokenManagerI extraction
- [ ] M2 BackendFetcher trait + HttpBackend wrapper + MockClient extension
- [ ] M3 Migrate services + handlers + delete old per-resource traits
- [ ] M4 Test consolidation + new HttpBackend integration tests
- [ ] M5 Preflight cleanup + covgate ratchet

Add timestamps and split entries as work proceeds.

## Surprises & Discoveries

(Add entries as work proceeds.)

## Decision Log

All entries below are dated 2026-04-08, authored by the plan author.

- Single `BackendFetcher` trait with three methods instead of three per-resource traits. All three fetchers share the same dependencies (HTTP client + token manager), auth, retry, and error-mapping flow. Reverses a prior plan's choice.
- One production wrapper `HttpBackend<'a, C: http::ClientI, T: authn::TokenManagerI>` holding borrows `{ client: &'a C, token_mngr: &'a T }`. Borrow-based fields preserve the current handler construction pattern.
- Introduce `authn::TokenManagerI` as a sibling of `authn::TokenManagerExt`, not a replacement. `TokenManagerExt` is the multi-threaded actor command interface (Shutdown, GetToken, RefreshToken) at `agent/agent/src/authn/token_mngr.rs:40-45` and `mod.rs:7`; it must stay untouched. `TokenManagerI` is a smaller fetch-only seam for unit-testing `HttpBackend`.
- `StubTokenManager` lives in `agent/agent/tests/test_utils/token_manager.rs`. `test_utils/mod.rs` currently declares only `pub mod testdata;`.
- `cache_miss_err` helper moves to `services/backend.rs` as `pub(crate) fn`. The new module removes the earlier "no cross-service shared module" constraint that justified duplication.
- One shared `StubBackend` and one `PanicBackend` fixture in `tests/services/backend_stub.rs`, replacing the three per-resource stubs.
- Handlers construct `HttpBackend` inline per request and pass `Some(&backend)`. Same per-handler pattern used today for the per-resource fetchers.
- TokenManagerI's method is named `current_token`, not `get_token`. Avoids E0034 ambiguous method errors when both `TokenManagerExt` and `TokenManagerI` are in scope, and avoids the recursive impl-body trap (`self.get_token().await` inside `impl TokenManagerI for TokenManager`).
- Ratchet `agent/agent/src/services/.covgate` up to within ~0.1% of the new actual coverage after M4 lands. Gate currently at `93.59`; M4 integration tests are expected to push it close to or above 98. Ratchet in M5 uses the real measured value.

## Outcomes & Retrospective

(Add entries as work proceeds.)

## Context and Orientation

The agent is a Rust service. Relevant paths (relative to the `agent/` submodule root):

- `agent/src/services/mod.rs` — 7 lines: declares submodules `deployment`, `device`, `errors`, `events`, `git_commit`, `release`; re-exports `ServiceErr` from `errors`. Refactor adds `pub mod backend;` and `pub use self::backend::{BackendFetcher, HttpBackend};`.
- `agent/src/services/deployment/get.rs`, `agent/src/services/release/get.rs`, `agent/src/services/git_commit/get.rs` — each defines a per-resource fetcher trait, a per-resource `Http*Fetcher` production wrapper, the service `get` function with cache-miss fallback, and a private `cache_miss_err` helper.
- `agent/src/services/.covgate` — currently `93.59`. Ratcheted in M5.
- `agent/src/authn/token_mngr.rs:259` — `pub async fn get_token(&self) -> Result<Arc<Token>, AuthnErr>` (defined on `TokenManagerExt`, not inherent). The signature `TokenManagerI` mirrors.
- `agent/src/authn/token.rs:8-12` — `pub struct Token { pub token: String, pub expires_at: DateTime<Utc> }`.
- `agent/src/authn/errors.rs:46-50,59-76` — `AuthnErr`. Simplest constructible variant is `MockError(MockError { is_network_conn_err: bool, trace: Box<Trace> })`, used by `StubTokenManager` for error cases.
- `agent/src/authn/token_mngr.rs:40-45` and `agent/src/authn/mod.rs:7` — `TokenManagerExt` (actor command interface). Do not modify.
- `agent/src/http/client.rs:30` — `pub trait ClientI: Send + Sync { fn base_url(&self) -> &str; fn execute(...) -> impl Future<Output = Result<(String, request::Meta), HTTPErr>> + Send; }`. Already implemented by production `http::Client` and by `tests::http::mock::MockClient`.
- `agent/src/http/deployments.rs:87-97` — `pub async fn get(client: &impl ClientI, id: &str, expansions: &[&str], token: &str) -> Result<Deployment, HTTPErr>`. Deployment fetch uses `expand=config_instances` only. The earlier `release.git_commit` expansion was removed in commit `a598cda` along with the expanded re-caching feature; do not re-introduce it.
- `agent/src/http/releases.rs:7-17` — same shape, returns `Release`.
- `agent/src/http/git_commits.rs:7-17` — same shape, returns `GitCommit`.
- `agent/src/http/retry.rs:26-30` — `pub async fn with_retry<F, Fut, T, E>(f: F) -> Result<T, E>` retries only on network errors.
- `agent/src/server/handlers.rs` — constructs the per-resource fetchers today and calls the service `get` functions. Refactor replaces those with a single `HttpBackend`.
- `agent/tests/http/mock.rs:204-231` — `MockClient` implements `ClientI`; has `Call::GetDeployment` + `get_deployment_fn` scaffolding. Refactor adds `Call::GetRelease`, `Call::GetGitCommit`, and matching `get_*_fn` fields and setters.
- `agent/tests/test_utils/mod.rs` — 1 line: `pub mod testdata;`. Refactor adds `pub mod token_manager;`.
- `agent/tests/services/mod.rs` — 6 lines declaring `deployment`, `device`, `errors`, `events`, `git_commit`, `release`. Refactor adds `pub mod backend;` and `pub mod backend_stub;`.
- `agent/tests/services/{deployment,release,git_commit}/get.rs` — each defines a per-resource `Stub<Resource>Fetcher` and `Panic<Resource>Fetcher` at file top level, with a `<resource>_fallback` test module. Refactor deletes those stubs and points the tests at the shared `StubBackend`/`PanicBackend`.

Terms of art:

- **Cache-miss fallback.** When a local-storage read misses, the service calls the backend, re-caches the result, and returns it. On 404 or token failure the service returns a "not found" error so upstream treats it as a cold cache miss.
- **covgate.** A plain-text minimum-coverage floor file checked by the preflight script. `agent/src/services/.covgate` currently contains `93.59`.
- **TokenManagerExt.** Agent's actor-style command interface for the token manager. Unrelated to the new seam. Do not modify.
- **Preflight.** `./scripts/preflight.sh` from the `agent/` root runs fmt, clippy, build, tests, and covgate checks.

Current cache-miss fallback flow (present today in each of the three `get.rs` files, to be preserved exactly):

1. `read_optional(id.clone())` → if `Some(v)` return it; if `None` and no backend provided, return `cache_miss_err`.
2. Otherwise `backend.fetch_<resource>(&id).await` and match:
   - `Ok(v)` → re-cache (`resolve_dpl` + write for deployment; `write_if_absent` for release/git_commit) and return `v`.
   - `Err(ServiceErr::SyncErr(sync::SyncErr::AuthnErr(_)))` → return `cache_miss_err`.
   - `Err(ServiceErr::HTTPErr(http::HTTPErr::RequestFailed(RequestFailed { status, .. })))` where `status == NOT_FOUND` → return `cache_miss_err`.
   - `Err(other)` → propagate.

## Plan of Work

Five milestones, each ending with one commit.

### M1 — TokenManagerI extraction

Add `authn::TokenManagerI` next to existing definitions in `agent/src/authn/token_mngr.rs` (or a small new file such as `agent/src/authn/token_mngr_iface.rs` if that keeps the file tidy):

    #[allow(async_fn_in_trait)]
    pub trait TokenManagerI: Send + Sync {
        async fn current_token(&self) -> Result<std::sync::Arc<Token>, AuthnErr>;
    }

Implement it for the real `TokenManager` by delegating through `TokenManagerExt` (`get_token` is defined on the trait, not inherent — see `agent/src/authn/token_mngr.rs:259`):

    impl TokenManagerI for TokenManager {
        async fn current_token(&self) -> Result<std::sync::Arc<Token>, AuthnErr> {
            <Self as TokenManagerExt>::get_token(self).await
        }
    }

Re-export `TokenManagerI` from `agent/src/authn/mod.rs` in the same style as the existing `TokenManagerExt` re-export. Do not touch `TokenManagerExt`. M1 adds no consumers.

### M2 — BackendFetcher trait, HttpBackend wrapper, MockClient extension

Create `agent/src/services/backend.rs`:

    #[allow(async_fn_in_trait)]
    pub trait BackendFetcher: Send + Sync {
        async fn fetch_deployment(&self, id: &str) -> Result<backend_client::Deployment, ServiceErr>;
        async fn fetch_release(&self, id: &str) -> Result<backend_client::Release, ServiceErr>;
        async fn fetch_git_commit(&self, id: &str) -> Result<backend_client::GitCommit, ServiceErr>;
    }

    pub struct HttpBackend<'a, C: http::ClientI, T: authn::TokenManagerI> {
        client: &'a C,
        token_mngr: &'a T,
    }

    impl<'a, C: http::ClientI, T: authn::TokenManagerI> HttpBackend<'a, C, T> {
        pub fn new(client: &'a C, token_mngr: &'a T) -> Self { Self { client, token_mngr } }
    }

The `impl BackendFetcher for HttpBackend` block mirrors current per-resource wrapper bodies: resolve a token via `self.token_mngr.current_token().await.map_err(|e| ServiceErr::SyncErr(sync::SyncErr::from(e)))?`, then call `http::with_retry(|| async { http::<resource>::get(self.client, id, <expansions>, &token.token).await }).await.map_err(ServiceErr::from)`. Expansions: `&["config_instances"]` for deployment, `&[]` for release and git_commit.

Add the helper (body copied verbatim from one current per-file helper):

    pub(crate) fn cache_miss_err(id: &str, kind: &str) -> ServiceErr { /* same body as today */ }

Update `agent/src/services/mod.rs` to add `pub mod backend;` and `pub use self::backend::{BackendFetcher, HttpBackend};`.

Extend `agent/tests/http/mock.rs` to support release and git_commit routes, mirroring the existing `GetDeployment` shape:

- Add `Call::GetRelease` and `Call::GetGitCommit` variants to the `Call` enum.
- Add `get_release_fn: SingleReleaseFn` and `get_git_commit_fn: SingleGitCommitFn` fields to `MockClient` with matching type aliases (mirror `SingleDeploymentFn`). Defaults return `BackendRelease::default()` / `BackendGitCommit::default()`.
- Add `set_get_release` and `set_get_git_commit` setters.
- Add new arms next to the existing `GetDeployment` arm:

        (GET, p) if p.starts_with("/releases/") => Call::GetRelease,
        (GET, p) if p.starts_with("/git_commits/") => Call::GetGitCommit,

- Add `handle_route` arms that invoke the new fns.

Nothing outside `src/services/backend.rs`, `src/services/mod.rs`, and `tests/http/mock.rs` changes in this milestone.

### M3 — Migrate services + handlers + delete old per-resource traits

For each of `agent/src/services/deployment/get.rs`, `agent/src/services/release/get.rs`, `agent/src/services/git_commit/get.rs`:

- Delete the `pub trait <Resource>Fetcher` definition.
- Delete the `pub struct Http<Resource>Fetcher<'a>` and its `impl <Resource>Fetcher` block.
- Delete the private `cache_miss_err` at the bottom.
- Change the service `get` signature from

        pub async fn get<F: <Resource>Fetcher>(<storage>, backend: Option<&F>, id: String) -> Result<models::<Resource>, ServiceErr>

  to

        pub async fn get<B: BackendFetcher>(<storage>, backend: Option<&B>, id: String) -> Result<models::<Resource>, ServiceErr>

- Update the call inside `get` to use `backend.fetch_<resource>(&id).await`.
- Route `cache_miss_err` via `use super::backend::cache_miss_err;` (or the crate path).
- In deployment's file, keep `resolve_dpl` exposed; only the fetcher glue is removed.

Update `agent/src/server/handlers.rs` to build one `HttpBackend` per request and pass `Some(&backend)` to each service `get` call. Delete the construction of the three per-resource `Http*Fetcher`s.

After this milestone, `cargo build --features test` against `src/` compiles cleanly, but the test crate will NOT compile because the three service test files still reference the deleted per-resource stubs/traits. This is expected. Commit anyway — M4 fixes the tests. Keeps source and test churn in distinct, reviewable commits.

### M4 — Test consolidation + new HttpBackend integration tests

Create `agent/tests/services/backend_stub.rs` (declared from `tests/services/mod.rs` via `pub mod backend_stub;`):

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use miru_agent::services::BackendFetcher;
    use miru_agent::services::ServiceErr;
    use backend_client::{Deployment, Release, GitCommit};

    pub struct StubBackend {
        deployment_result: Mutex<Option<Result<Deployment, ServiceErr>>>,
        release_result: Mutex<Option<Result<Release, ServiceErr>>>,
        git_commit_result: Mutex<Option<Result<GitCommit, ServiceErr>>>,
        deployment_calls: AtomicUsize,
        release_calls: AtomicUsize,
        git_commit_calls: AtomicUsize,
    }

    impl StubBackend {
        pub fn new() -> Self { /* zero counts, None slots */ }
        pub fn with_deployment(self, r: Result<Deployment, ServiceErr>) -> Self { /* set slot, return self */ }
        pub fn with_release(self, r: Result<Release, ServiceErr>) -> Self { /* ... */ }
        pub fn with_git_commit(self, r: Result<GitCommit, ServiceErr>) -> Self { /* ... */ }
        pub fn deployment_calls(&self) -> usize { self.deployment_calls.load(Ordering::SeqCst) }
        pub fn release_calls(&self) -> usize { self.release_calls.load(Ordering::SeqCst) }
        pub fn git_commit_calls(&self) -> usize { self.git_commit_calls.load(Ordering::SeqCst) }
    }

    impl BackendFetcher for StubBackend {
        async fn fetch_deployment(&self, _id: &str) -> Result<Deployment, ServiceErr> {
            self.deployment_calls.fetch_add(1, Ordering::SeqCst);
            self.deployment_result.lock().unwrap().take().expect("no canned deployment")
        }
        async fn fetch_release(&self, _id: &str) -> Result<Release, ServiceErr> { /* mirror */ }
        async fn fetch_git_commit(&self, _id: &str) -> Result<GitCommit, ServiceErr> { /* mirror */ }
    }

    pub struct PanicBackend;

    impl BackendFetcher for PanicBackend {
        async fn fetch_deployment(&self, _id: &str) -> Result<Deployment, ServiceErr> { panic!("PanicBackend::fetch_deployment called"); }
        async fn fetch_release(&self, _id: &str) -> Result<Release, ServiceErr> { panic!("PanicBackend::fetch_release called"); }
        async fn fetch_git_commit(&self, _id: &str) -> Result<GitCommit, ServiceErr> { panic!("PanicBackend::fetch_git_commit called"); }
    }

Create `agent/tests/test_utils/token_manager.rs` (declared from `tests/test_utils/mod.rs` via `pub mod token_manager;`):

    use std::sync::{Arc, Mutex};
    use miru_agent::authn::{AuthnErr, Token, TokenManagerI};

    /// One-shot test stub: each call to `current_token` consumes the canned response.
    /// `with_retry` does not re-fetch the token, so this is sufficient for current tests.
    /// Calling `current_token` more than once panics with "no canned response".
    pub struct StubTokenManager {
        result: Mutex<Option<Result<Arc<Token>, AuthnErr>>>,
    }

    impl StubTokenManager {
        pub fn ok(token: &str) -> Self { /* build Arc<Token> with expires_at = now + 1h */ }
        pub fn err(e: AuthnErr) -> Self { Self { result: Mutex::new(Some(Err(e))) } }
    }

    impl TokenManagerI for StubTokenManager {
        async fn current_token(&self) -> Result<Arc<Token>, AuthnErr> {
            self.result.lock().unwrap().take().expect("no canned token response")
        }
    }

Refactor `agent/tests/services/deployment/get.rs`, `agent/tests/services/release/get.rs`, `agent/tests/services/git_commit/get.rs`:

- Delete the per-resource `Stub<Resource>Fetcher` and `Panic<Resource>Fetcher`.
- Replace fallback-test construction sites with `StubBackend::new().with_<resource>(Ok(synthetic_value))` (or `Err(synthetic_error)`), and call `svc::get(&storage, Some(&backend), id).await`.
- Replace cache-hit test backends with `&PanicBackend`.
- Replace no-backend test sites (`get_deployment::*`, `get_current_deployment::*`, `get_release::*`, `get_current_release::*`, `get_git_commit::*`) with `None::<&PanicBackend>` at every no-backend site — makes intent explicit: if the None branch is taken, nothing should call through.
- Keep every existing test's name, assertions, and storage helpers unchanged. Keep `resolve_dpl_none_cached_returns_new`, `resolve_dpl_cached_preserves_local_state_and_takes_new_target`, and `cache_miss_backend_missing_config_instances_returns_sync_err` (deployment-only).
- Leave the `setup` helpers as-is.

Create `agent/tests/services/backend.rs` (declared from `tests/services/mod.rs` via `pub mod backend;`). Integration tests driving `HttpBackend` with `MockClient` + `StubTokenManager`:

- `fetch_deployment_constructs_url_and_expand_param` — call `HttpBackend::fetch_deployment("dpl_1")`, assert MockClient captured `(GET, "/deployments/dpl_1")` with query `("expand", "config_instances")`.
- `fetch_deployment_returns_deserialized_value` — MockClient returns synthetic `Deployment`; assert match.
- `fetch_release_constructs_url_no_expand` — assert `(GET, "/releases/rls_1")`, no expand.
- `fetch_release_returns_deserialized_value` — happy path.
- `fetch_git_commit_constructs_url_no_expand` — assert `(GET, "/git_commits/gc_1")`, no expand.
- `fetch_git_commit_returns_deserialized_value` — happy path.
- `fetch_deployment_token_failure_returns_sync_err` — `StubTokenManager::err(AuthnErr::MockError(...))`; assert `Err(ServiceErr::SyncErr(SyncErr::AuthnErr(_)))`.
- `fetch_deployment_404_propagates_as_request_failed` — MockClient returns 404; assert `Err(ServiceErr::HTTPErr(HTTPErr::RequestFailed(RequestFailed { status: NOT_FOUND, .. })))`.
- `fetch_deployment_5xx_propagates_as_request_failed` — MockClient returns 500; assert similar with `INTERNAL_SERVER_ERROR`.
- `fetch_deployment_with_retry_recovers_from_network_error` — MockClient returns network error on call 1, success on call 2; assert eventual success and call count == 2.

Add 404 / 5xx tests for release and git_commit only if they cover a meaningfully different code path.

After M4 the test suite compiles and all 1128 existing tests plus the new ones pass.

### M5 — Preflight cleanup + covgate ratchet

From `agent/`, run `./scripts/preflight.sh`. Fix any fmt/clippy/lint drift. Read the reported services coverage; ratchet `agent/agent/src/services/.covgate` up to within ~0.1% of the new actual value (likely close to or above 98). Re-run preflight until it reports `clean`.

Do NOT touch `agent/agent/src/authn/.covgate` unless M1 genuinely moves the authn number and preflight asks for a ratchet.

## Concrete Steps

All commands run from the `agent/` submodule root unless stated otherwise. Each milestone's edits are described in Plan of Work; the commands below are the exact build/test/commit sequence per milestone.

### M1

    cargo build --features test
    git add -A
    git commit -m "feat(authn): extract TokenManagerI trait for service-layer testing"

### M2

    cargo build --features test
    cargo build --features test --tests
    git add -A
    git commit -m "feat(services): add BackendFetcher trait and HttpBackend wrapper"

### M3

    cargo build --features test
    cargo build --features test --tests
    git add -A
    git commit -m "refactor(services): consolidate cache-miss fallback under single BackendFetcher trait"

Library build is clean. The tests build is expected to FAIL with errors pointing at the three service test files that still reference the deleted per-resource fetchers/stubs. Intended; do not fix here.

### M4

    cargo build --features test --tests
    cargo test --features test
    git add -A
    git commit -m "test(services): consolidate stubs and add HttpBackend integration tests"

All tests (existing + new) must pass; fix any failures inside this milestone.

### M5

    ./scripts/preflight.sh
    # edit agent/src/services/.covgate to within ~0.1% of reported services coverage
    ./scripts/preflight.sh
    git add -A
    git commit -m "chore(preflight): satisfy lint and ratchet services covgate post-refactor"

Expect a `clean` result on the second preflight run.

## Validation and Acceptance

Behavior: there is none to change. Acceptance = structural + test outcomes.

1. Run `cargo test --features test` from `agent/`. Expect all 1128 pre-existing tests to still pass, plus the new `tests/services/backend.rs` integration tests.
2. preflight (./scripts/preflight.sh or the agent's preflight checks invoked through $preflight) must report `clean` before a PR is opened. This is a hard gate enforced by the orchestrator.
3. Run `git diff --stat fix/cache-backups...HEAD` from `agent/`. Expect a negative net change across source files.
4. Confirm `agent/src/services/mod.rs` contains `pub mod backend;` and `pub use self::backend::{BackendFetcher, HttpBackend};`.
5. Confirm `agent/src/services/.covgate` has been ratcheted above `93.59` to within ~0.1% of the new actual services coverage.
6. Confirm grep does NOT find any of `DeploymentFetcher`, `ReleaseFetcher`, `GitCommitFetcher`, `HttpDeploymentFetcher`, `HttpReleaseFetcher`, `HttpGitCommitFetcher`, `StubDeploymentFetcher`, `StubReleaseFetcher`, `StubGitCommitFetcher`, `PanicDeploymentFetcher`, `PanicReleaseFetcher`, `PanicGitCommitFetcher` anywhere in `agent/`.

## Out of scope

- Any change to user-visible behavior.
- Re-introducing the expanded release/git_commit re-caching feature removed in commit `a598cda`.
- Any change to `get_current` for any resource.
- Any change to the OpenAPI spec.
- Any change to `libs/backend-api` or `libs/device-api` (generated).
- Any change to `authn::TokenManagerExt` (actor command interface).
- Any change to PR #24's commit history.

## Idempotence and Recovery

- M1, M2, M4, M5 are idempotent: additive edits; re-run builds/preflight as needed.
- M3 intentionally leaves the test crate broken between M3 and M4. If interrupted after M3, resume into M4 — do NOT try to make `cargo test` pass at the M3 commit. If M3 must be abandoned, `git reset --hard HEAD~1` from `agent/` removes it without affecting M1/M2; only do this if explicitly approved, since the branch is stacked and rewriting history has stacking implications.
- Send-bound fallback: if M3 fails to compile because the future returned by `BackendFetcher::fetch_*` is not `Send` when called from an axum handler closure, the fix is to change the trait declaration to use the `trait-variant` crate's `#[trait_variant::make(BackendFetcher: Send)]` attribute, OR add explicit `+ Send` bounds via the `impl Future<Output = ...> + Send` desugaring. The pre-M3 per-resource `DeploymentFetcher` trait (which this refactor deletes) already used native `async fn` in trait without ceremony and worked through axum, so this is not expected to fire — workaround is documented in case the generic `B: BackendFetcher` form trips a different inference path.

If preflight reports a covgate that drops below the ratcheted floor on a later run (e.g. due to a fix that removed a test), lower the floor only to within ~0.1% of the new measured value and note the reason in the Decision Log.
