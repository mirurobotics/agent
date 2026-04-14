# Update tests for `release/current.rs` after backend fallback refactor

**Status**: backlog

## Goal

Update the test file `agent/tests/services/release/current.rs` to match the
new signature of `rls_svc::get_current`, which now accepts a `&impl BackendFetcher`
parameter. The implementation changed from a direct cache read
(`releases.read(dpl.release_id)`) to a cache-then-backend fallback
(`rls_svc::get(releases, backend, dpl.release_id)`), so tests must cover both
the cache-hit path (no backend call) and the cache-miss path (backend called).

## Scope

### Test changes (1 file)

- `agent/tests/services/release/current.rs`

All changes are confined to this single test file. No source changes.

## Context

### Signature change

```rust
// Before
pub async fn get_current(
    deployments: &storage::Deployments,
    releases: &storage::Releases,
) -> Result<models::Release, ServiceErr>

// After
pub async fn get_current(
    deployments: &storage::Deployments,
    releases: &storage::Releases,
    backend: &impl BackendFetcher,
) -> Result<models::Release, ServiceErr>
```

### Behavior change

- Cache hit: returns immediately, no backend call (unchanged).
- Cache miss: falls back to `rls_svc::get(releases, backend, dpl.release_id)`,
  which calls `backend.fetch_release(&id)`. Backend errors now propagate
  through `get_current`.

### Test infrastructure

- `PanicBackend` (`tests/services/backend_stub.rs`) -- panics if any fetch
  method is called. Use in tests where the backend must not be consulted.
- `StubBackend` (`tests/services/backend_stub.rs`) -- canned results via
  `.with_release(Ok(..))` or `.with_release(Err(..))`, tracks call counts
  via `.release_calls()`.
- Error construction patterns are established in `tests/services/release/get.rs`
  and `tests/services/deployment/get.rs`.

## Steps

### M1 -- Update existing tests to pass `backend` parameter

Update the `setup` function and all four existing tests to pass a backend
parameter to `rls_svc::get_current`.

**1. Add imports** for `PanicBackend`, `StubBackend`, and backend error types.
The import block should become:

```rust
// internal crates
use crate::services::backend_stub::{PanicBackend, StubBackend};
use backend_api::models as backend_client;
use miru_agent::filesys::{self, Overwrite};
use miru_agent::http::errors::{HTTPErr, RequestFailed};
use miru_agent::http::request::Params as HttpParams;
use miru_agent::models::{Deployment, DplActivity, DplErrStatus, DplTarget, Release};
use miru_agent::services::release as rls_svc;
use miru_agent::services::ServiceErr;
use miru_agent::storage::{Deployments, Releases};
use miru_agent::sync::SyncErr;
use miru_agent::authn::errors::{AuthnErr, MockError as AuthnMockError};

// external crates
use chrono::{DateTime, Utc};
```

**2. Update `returns_release_for_deployed_deployment`** (test 1):
- Pass `&PanicBackend` as third arg to `rls_svc::get_current`.
- Release is in cache, so `PanicBackend` proves no backend call is made.

**3. Update `no_deployed_deployment_returns_error`** (test 2):
- Pass `&PanicBackend` as third arg.
- No deployments in cache, so `get_current` errors before reaching
  the release lookup. `PanicBackend` proves this.

**4. Update `multiple_deployed_deployments_returns_error`** (test 4):
- Pass `&PanicBackend` as third arg.
- Multiple deployed deployments error before release lookup.

**5. Rewrite `deployed_deployment_with_missing_release_returns_error`** (test 3):
- Previously: release not in cache, expected `Err(ServiceErr::CacheErr(_))`.
- Now: the code falls back to the backend. The test must supply a `StubBackend`
  that returns a 404 error so the test still exercises the "release not found" path.
- Change the backend to:
  ```rust
  let err = ServiceErr::HTTPErr(HTTPErr::RequestFailed(RequestFailed {
      request: HttpParams::get("http://test/cache-miss").meta().unwrap(),
      status: reqwest::StatusCode::NOT_FOUND,
      error: None,
      trace: miru_agent::trace!(),
  }));
  let stub = StubBackend::new().with_release(Err(err));
  ```
- Change the assertion from `Err(ServiceErr::CacheErr(_))` to
  `Err(ServiceErr::HTTPErr(HTTPErr::RequestFailed(_)))`.
- Pass `&stub` as third arg.

Commit: `test(release): update get_current tests for backend param`

### M2 -- Add new test cases

Add a new `mod get_current_release_fallback` block (inside the same file,
after the existing `mod get_current_release`) with these tests:

**1. `release_cached_no_backend_call`**
- Deployment in cache (Deployed), release in cache.
- Pass `&PanicBackend`.
- Assert success, verify returned release matches.
- Purpose: explicit cache-hit proof with named `PanicBackend`.

**2. `release_not_cached_backend_returns_release`**
- Deployment in cache (Deployed), release NOT in cache.
- `StubBackend` returns `Ok(backend_client::Release { id, version, .. })`.
- Assert success, verify `stub.release_calls() == 1`.
- Purpose: proves the fallback path through `rls_svc::get` works end-to-end
  when called from `get_current`.

**3. `release_not_cached_backend_404_returns_error`**
- Deployment in cache (Deployed), release NOT in cache.
- `StubBackend` returns `Err(ServiceErr::HTTPErr(HTTPErr::RequestFailed(...)))` with 404.
- Assert `Err(ServiceErr::HTTPErr(HTTPErr::RequestFailed(_)))`.
- Purpose: 404 from backend propagates correctly.

**4. `release_not_cached_backend_token_err_returns_error`**
- Deployment in cache (Deployed), release NOT in cache.
- `StubBackend` returns `Err(ServiceErr::SyncErr(SyncErr::AuthnErr(AuthnErr::MockError(...))))`.
- Assert `Err(ServiceErr::SyncErr(SyncErr::AuthnErr(_)))`.
- Purpose: token/auth failure propagates correctly.

All new tests reuse the existing `setup` function (which creates temp dir +
`Deployments` + `Releases` storage).

Commit: `test(release): add get_current backend fallback tests`

### M3 -- Run tests and coverage

1. Run `./scripts/test.sh` from `agent/` repo root. All tests must pass.
2. Run `./scripts/covgate.sh`. If coverage improved, ratchet the `.covgate`
   file for the `services::release` module.

Commit (if ratchet): `chore(coverage): ratchet release service coverage`

## Test steps

- `./scripts/test.sh` -- full test suite passes
- `cargo test -p miru-agent --test mod --features test -- services::release::current` -- focused run on changed tests
- `./scripts/covgate.sh` -- coverage gates pass

## Validation

Preflight must report `clean` before publishing.
