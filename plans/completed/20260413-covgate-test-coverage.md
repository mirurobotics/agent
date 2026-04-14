# Improve test coverage of new/updated covgates in services layer

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` | read-write | Add tests in `agent/tests/services/`, ratchet `.covgate` files in `agent/src/services/` |

This plan lives in `agent/plans/backlog/` because all changes are within the agent repo.

## Purpose / Big Picture

The `refactor/services-backend-fetcher` branch consolidated three per-resource backend fetcher traits into a single `BackendFetcher` trait. This introduced new per-module `.covgate` files and updated the parent services gate. Several coverage gaps remain:

1. **`backend.rs` error paths are only tested for `fetch_deployment`** -- the token-failure, HTTP-error, and retry-recovery paths for `fetch_release` and `fetch_git_commit` are structurally identical but lack coverage.
2. **Cache write error branches are untested** in all three `get.rs` files (`deployment/get.rs`, `release/get.rs`, `git_commit/get.rs`). The `if let Err(e) = ... .write(...).await { error!(...) }` branch is never reached.
3. **Deployment dirty-flag predicate** (`|old, _| old.is_some_and(|e| e.is_dirty)`) is never evaluated with `old = Some(dirty_entry)`, so the skip-overwrite path is untested.

After this plan, all `.covgate` thresholds should increase and the coverage gaps above should be closed (or documented as infeasible).

## Current Thresholds

| Module | Threshold | Key Gaps |
|--------|-----------|----------|
| `services/` (parent) | 94.72% | `backend.rs`: token failure + HTTP errors only tested through `fetch_deployment` |
| `services/deployment/` | 93.05% | `get.rs`: cache write error; dirty-flag predicate with `old = Some(dirty)` |
| `services/release/` | 90.9% | `get.rs`: cache write error |
| `services/git_commit/` | 88.09% | `get.rs`: cache write error |
| `services/device/` | 96.07% | Minor gaps -- low priority, out of scope |
| `services/events/` | 100% | No gaps -- out of scope |

## Progress

- [x] Move plan to `plans/active/`
- [x] Milestone 1: Identify exact coverage gaps (run `covgate.sh` and optionally `cargo llvm-cov --html`)
- [x] Milestone 2: Add `backend.rs` error-path tests for `fetch_release` and `fetch_git_commit`
- [x] Milestone 3: Investigate and add cache write error tests for all three `get.rs` files
- [x] Milestone 4: Test the deployment dirty-flag predicate
- [x] Milestone 5: Re-run `covgate.sh`, verify improvement, and ratchet `.covgate` files upward
- [ ] Validation: preflight must report `clean` before publishing

## Surprises & Discoveries

- The dirty-flag predicate test does not exercise the production `get()` code path (only reachable via a concurrent write race), but it validates the closure logic directly through the storage layer, which is the best achievable without production code changes.
- The sub-module covgates (`services/deployment`, `services/release`, `services/git_commit`) did not increase because the new backend.rs tests only exercise code in `backend.rs` (under the parent `services/` module). The dirty-flag test exercises `storage` module code.
- Cache write error branches remain untested (Option A chosen) -- documented with comments in all three `get.rs` test files.

## Decision Log

- **Cache write errors (Milestone 3)**: Chose Option A (skip and document). The `shutdown()` approach breaks `read_optional()` first, making write errors unreachable through the public `get()` API. No clean mock approach exists under ~50 lines of infrastructure. Added comments in all three test files.

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

### `backend.rs` (70 lines)

Defines `BackendFetcher` trait with `fetch_deployment`, `fetch_release`, `fetch_git_commit`. The `HttpBackend` impl calls `self.token().await?` then `http::with_retry(|| http::*.get(...))`. Token failure and HTTP error are separate code regions per method. All three methods are structurally identical, but only `fetch_deployment` has error-path and retry tests.

### `{deployment,release,git_commit}/get.rs`

Each has a public `get()` function (cache-hit returns early; cache-miss calls backend then `cache_*()`) and a private `cache_*()` that writes to storage. The write error branch (`if let Err(e) = storage.write(...).await { error!(...) }`) is untested in all three.

Deployment's `cache_deployment` additionally passes a dirty-flag predicate: `|old, _| old.is_some_and(|e| e.is_dirty)`. This predicate is evaluated by the storage actor's `write()` path to decide whether the new entry should be marked dirty. It is never tested with `old = Some(entry_where_is_dirty_is_true)`.

### Test infrastructure

- `StubBackend` and `PanicBackend` in `agent/tests/services/backend_stub.rs`: canned results for `BackendFetcher` methods.
- `StubTokenManager` in `agent/tests/test_utils/token_manager.rs`: returns a canned token or error.
- `MockClient` in `agent/tests/http/mock.rs`: closure-based mock for `ClientI` with per-endpoint overrides (`set_get_deployment`, `set_get_release`, `set_get_git_commit`).
- `FileCache::spawn()` returns `(Self, JoinHandle<()>)`. The `JoinHandle` is the actor task. Aborting or shutting down the actor causes subsequent `write()` calls to return `CacheErr::SendActorMessageErr` -- this is the mechanism for inducing cache write errors.

### How to induce cache write errors

The `ConcurrentCache` (which `FileCache` wraps) communicates with its actor via an `mpsc::Sender`. When the actor is shut down or its `JoinHandle` is aborted, the channel closes. A subsequent `.write()` call will fail in `send_command` with `CacheErr::SendActorMessageErr`.

Strategy:
1. Call `storage.shutdown().await` to cleanly stop the actor.
2. Then call the service's `get()` with a cache-miss (so it hits the backend and tries to cache).
3. The backend returns Ok, the service calls `cache_*()`, and `write()` returns `Err(SendActorMessageErr)`.
4. The `if let Err(e)` branch executes, logging the error.
5. The service still returns `Ok(value)` because the cache write error is swallowed.

Important: after `shutdown()`, the `read_optional()` call at the top of `get()` will *also* fail. Since `read_optional` returns `Result<Option<V>, CacheErr>` and the error propagates with `?`, the `get()` function will return `Err(ServiceErr::CacheErr(...))` before ever reaching the backend. This means we **cannot** induce a write error through the public `get()` API using `shutdown()` alone.

Alternative approach: drop the `JoinHandle` after a successful `read_optional` but before `write`. This is not feasible without modifying production code.

**Conclusion on cache write error tests**: the cache write error branch in `cache_*()` is not reachable through the public `get()` API using the current test infrastructure. To test it, one would need either:
- A storage wrapper that returns Ok for reads but Err for writes (a new mock/stub layer), or
- Testing `cache_*()` directly, but it is `pub(crate)` -- not accessible from integration tests.

This gap should be documented and the branch left untested unless a lightweight mock storage layer is added. The branch is a defensive log-and-continue pattern, so the risk of it being incorrect is low.

### How to test the dirty-flag predicate

The predicate `|old, _| old.is_some_and(|e| e.is_dirty)` is evaluated inside the storage actor when `cache_deployment` calls `write()`. It determines the `is_dirty` flag of the newly written cache entry. To test it:

1. Write a deployment to storage with `is_dirty: true`. The `CacheEntry` struct has an `is_dirty: bool` field, but the public `write()` method *computes* `is_dirty` using the closure. To get an entry that is dirty, we need to call `write()` with a closure that returns `true` (e.g., `|_, _| true`).
2. Then trigger `get()` with a cache-miss for the same deployment ID. Since a cached value exists, the cache-hit path returns early and the dirty-flag predicate is never reached.

Wait -- step 2 will return the cached value immediately. We need the cache to *not* have the entry for `read_optional` but *have* a dirty entry when `cache_deployment` calls `write`. That is a race condition we cannot control.

Alternative: use a StubBackend that returns a deployment with the *same ID* as an existing dirty entry, but force the `get()` path past the cache hit. This is impossible because `read_optional` will find the existing entry and return it.

**Revised approach**: The predicate fires when `cache_deployment` writes an entry whose key already exists in the cache. This can happen if a concurrent caller already cached the same deployment. However, in a test, we can:

1. Write a deployment entry using `deployments.write(id, dpl, |_, _| true, Overwrite::Allow)` to seed a dirty entry.
2. Call `dpl_svc::get()` with the same ID. Since the entry exists, `read_optional` returns it immediately and `cache_deployment` is never called.

This means the dirty-flag predicate is not reachable via the public `get()` path when the cache already has the entry. It only fires when a concurrent write sneaks in between `read_optional` (returns None) and `cache_deployment`. Testing this race requires either:
- A hook in the backend stub that writes to storage mid-flight, or
- A direct test of the predicate function itself (unit test for the closure logic).

**Practical approach**: test the *predicate logic* by calling `storage.write()` directly in the test with the same closure and asserting the resulting `is_dirty` field via `read_entry_optional`. This tests the semantic behavior even though it does not go through `dpl_svc::get()`. The `read_entry_optional` method returns `CacheEntry<K, V>` which exposes `is_dirty`.

## Plan of Work

### Milestone 1: Identify exact coverage gaps

**Why**: confirm the gaps listed above and discover any additional uncovered lines.

**Steps**:
1. Run `./scripts/covgate.sh` from the agent repo root and record per-module percentages.
2. Optionally run `cargo llvm-cov --html --features test -- --test-threads=1` and open the HTML report to inspect per-line coverage in `backend.rs`, `deployment/get.rs`, `release/get.rs`, `git_commit/get.rs`.

**Expected output**: the six `.covgate` modules all pass at their current thresholds; the HTML report shows uncovered lines matching the gaps in the table above.

### Milestone 2: Add `backend.rs` error-path tests

**Why**: `fetch_release` and `fetch_git_commit` have the same token-failure, HTTP-error, and retry-recovery code as `fetch_deployment`, but only `fetch_deployment` variants exist.

**File**: `agent/tests/services/backend.rs`

**Tests to add** (6 new tests):

```
fetch_release_token_failure_returns_sync_err
fetch_release_404_propagates_as_request_failed
fetch_release_with_retry_recovers_from_network_error
fetch_git_commit_token_failure_returns_sync_err
fetch_git_commit_404_propagates_as_request_failed
fetch_git_commit_with_retry_recovers_from_network_error
```

Each mirrors the existing deployment variant:

- **Token failure** (`fetch_release_token_failure_returns_sync_err`, `fetch_git_commit_token_failure_returns_sync_err`): Use `StubTokenManager::err(AuthnErr::MockError(...))`. Assert result is `Err(ServiceErr::SyncErr(SyncErr::AuthnErr(AuthnErr::MockError(_))))`. Assert `mock.requests().is_empty()`.

- **HTTP 404** (`fetch_release_404_propagates_as_request_failed`, `fetch_git_commit_404_propagates_as_request_failed`): Use `mock.set_get_release(|| Err(HTTPErr::RequestFailed(...)))` / `mock.set_get_git_commit(...)` with `StatusCode::NOT_FOUND`. Assert result matches `Err(ServiceErr::HTTPErr(HTTPErr::RequestFailed(rf)))` where `rf.status == NOT_FOUND`.

- **Retry recovery** (`fetch_release_with_retry_recovers_from_network_error`, `fetch_git_commit_with_retry_recovers_from_network_error`): Use `AtomicUsize` counter in the mock closure. First 2 calls return `Err(HTTPErr::MockErr(HttpMockErr { is_network_conn_err: true }))`, third returns `Ok(...)`. Assert success and `mock.call_count(Call::GetRelease) == 3` / `mock.call_count(Call::GetGitCommit) == 3`.

**Convention**: follow the existing import order (standard, internal, external) and test naming pattern.

### Milestone 3: Investigate and (conditionally) add cache write error tests

**Why**: the `if let Err(e) = storage.write(...) { error!(...) }` branch is untested in all three `get.rs` files.

**Investigation**: as documented in the Context section above, inducing a write error through the public `get()` API is not feasible with current infrastructure because `shutdown()` also breaks `read_optional()` which runs first. Two alternatives:

#### Option A: Skip and document (recommended if no lightweight mock exists)

If no clean way to inject a write-only failure is found, document the gap as accepted risk and move on. The branch is a defensive log-and-continue pattern that swallows the error -- the risk of it being incorrect is minimal.

Add a comment in each `get.rs` test file:

```rust
// NOTE: The cache write error branch in cache_* is not reachable through
// the public get() API in tests. Inducing a write failure (e.g. via
// shutdown) also breaks the read_optional() call that precedes it.
// The branch is a defensive log-and-continue pattern.
```

#### Option B: Add a FailingWriteStorage wrapper (only if the approach is clean)

Create a thin wrapper around `Deployments`/`Releases`/`GitCommits` that delegates `read_optional` normally but returns an error on `write`. This requires understanding whether the storage types are trait-based or concrete. Since `get()` is generic over `BackendFetcher` but takes concrete `storage::Deployments`, a wrapper would need to match the concrete type's interface.

**Decision**: during implementation, spend at most 30 minutes exploring Option B. If it requires more than ~50 lines of new infrastructure, take Option A.

### Milestone 4: Test the deployment dirty-flag predicate

**Why**: the predicate `|old, _| old.is_some_and(|e| e.is_dirty)` is never evaluated with `old = Some(dirty_entry)`.

**File**: `agent/tests/services/deployment/get.rs`

**Approach**: test the predicate logic directly via the storage layer, since the predicate is only reachable in `get()` during a concurrent write race that cannot be reliably reproduced.

**Test to add**:

```
cache_deployment_preserves_dirty_flag_on_overwrite
```

Steps:
1. Spawn a `Deployments` storage.
2. Write an entry with a closure that always returns `true` (`|_, _| true`) and `Overwrite::Allow`. This seeds a dirty entry.
3. Read back via `dpl_stor.read_entry_optional("dpl_1")` and assert `entry.is_dirty == true`.
4. Write the same key again using the *same closure as production code*: `|old, _| old.is_some_and(|e| e.is_dirty)`. Pass `Overwrite::Allow`.
5. Read back again and assert `entry.is_dirty == true` (the dirty flag was preserved because the old entry was dirty).
6. Additionally: write a *non-dirty* entry first (closure returns `false`), then overwrite with the production closure. Assert `entry.is_dirty == false` (non-dirty old entry is not marked dirty).

This directly validates the predicate logic even though it does not go through `dpl_svc::get()`.

### Milestone 5: Re-run covgate and ratchet thresholds

**Why**: verify that the new tests improved coverage and ratchet the gates upward to prevent regression.

**Steps**:
1. Run `./scripts/covgate.sh` and record the new percentages.
2. For each module where coverage increased, update the `.covgate` file to the new value (rounded down to 2 decimal places to avoid flaky gates from floating-point jitter).
3. Run `./scripts/covgate.sh` again to confirm all gates pass with the new thresholds.

**Files to update**:
- `agent/src/services/.covgate`
- `agent/src/services/deployment/.covgate`
- `agent/src/services/release/.covgate`
- `agent/src/services/git_commit/.covgate`

## Concrete Steps

All commands run from `/home/ben/miru/workbench3/agent/`.

1. `mv plans/backlog/20260413-covgate-test-coverage.md plans/active/`
2. `./scripts/covgate.sh` -- record baseline percentages
3. Optionally: `cargo llvm-cov --html --features test -- --test-threads=1 && open target/llvm-cov/html/index.html`
4. Edit `agent/tests/services/backend.rs` -- add 6 new tests (token failure, 404, retry for release and git_commit)
5. `./scripts/test.sh` -- verify all tests pass
6. Edit `agent/tests/services/deployment/get.rs` -- add dirty-flag predicate test
7. `./scripts/test.sh` -- verify all tests pass
8. Investigate cache write error feasibility (30-minute timebox); add tests or document gap
9. `./scripts/covgate.sh` -- record new percentages
10. Update `.covgate` files with new thresholds
11. `./scripts/covgate.sh` -- confirm all gates pass
12. `./scripts/test.sh` -- final verification
13. Commit via `$commit`

Expected output for step 5, 7, 12:

```
test result: ok. N passed; 0 failed; 0 ignored
```

Expected output for step 9, 11:

```
All coverage gates passed.
```

## Test Steps

### backend.rs error-path tests

For each of the 6 new tests:
1. Run `./scripts/test.sh` and confirm the test appears in the output and passes.
2. Temporarily break the assertion (e.g., change `NOT_FOUND` to `OK`) and confirm the test fails, proving it is not vacuous.

### dirty-flag predicate test

1. Run `./scripts/test.sh` and confirm `cache_deployment_preserves_dirty_flag_on_overwrite` passes.
2. Temporarily change the production predicate to `|_, _| false` and confirm the test fails (dirty flag is no longer preserved).

### Coverage verification

1. Run `./scripts/covgate.sh` after adding tests.
2. Confirm each affected module's coverage increased relative to baseline.
3. Confirm all gates pass after ratcheting.

## Validation and Acceptance

Before publishing any changes (PR or push):

- `./scripts/test.sh` passes with zero failures.
- `./scripts/covgate.sh` passes with all gates green at the ratcheted thresholds.
- `./scripts/lint.sh` passes with zero warnings.
- **Preflight must report `clean` before changes are published.** Do not open a PR or push if preflight has any failures.

Coverage thresholds should be strictly higher than or equal to the values in the "Current Thresholds" table above. The `services/device/` and `services/events/` gates must remain unchanged.

## Idempotence and Recovery

- All changes are additive (new test functions, updated threshold numbers). No production code is modified.
- If a test is flaky, it can be removed without affecting other tests.
- `.covgate` ratchets can be reverted by restoring the old threshold values.
- If the cache write error investigation proves infeasible (Option A), the only artifact is a comment in the test file -- no production code or infrastructure changes.
