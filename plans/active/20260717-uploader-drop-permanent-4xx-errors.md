# Stop the upload actor from retrying permanent 4xx backend errors

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (this repo, `mirurobotics/agent`) | read-write | Upload actor retry classification: `agent/src/upload/{errors,executor,uploader}.rs` plus tests under `agent/tests/upload/` and `agent/tests/mocks/`. |

This plan lives in `plans/backlog/` of the agent repo because all code changes are contained in this repository. Base branch: `main`. Working branch: `claude/task-mode-pr-agent-nxwei3`.

## Purpose / Big Picture

The agent (a Rust binary running on customer robots) uploads device files to cloud storage via an "upload actor". Each upload job first calls the Miru backend (`POST /agent/v1/uploads`) to register the upload and obtain scoped cloud credentials. Today, when that backend call fails with a **permanent** client error — observed in a staging device log on 2026-07-17 as a hard `404 resource_not_found` ("Upload Rule with id 'upl_col_NVbcMGwVyZapCSyuLhy1bijQn9suEdMRe' not found") — the actor retries the job 9 times over ~5 minutes with backoff before dropping it. Those retries can never succeed, waste device and backend resources, and delay other queued uploads.

After this change, an upload attempt that fails with a definitive 4xx from the upload's own backend request (e.g. 404; excluding 408 Request Timeout, 429 Too Many Requests, and 401 Unauthorized) is dropped after that single attempt with a clear, distinct log line. Transient failures — network connection errors, timeouts, 5xx, 408, 429, token-manager failures, and cloud-transfer failures — keep the existing retry/backoff/requeue behavior exactly as-is.

Observable outcome: with a mock executor scripted to return a permanent 404 error, the actor makes exactly 1 attempt, performs no backoff sleeps, does not requeue, logs `dropping upload job after 1 attempt(s) ... permanent client error, not retrying`, and immediately proceeds to the next queued job. With a transient error the actor still makes 9 attempts (3 rounds of 3) before dropping.

## Progress

- [x] Add `permanent` flag to `ExecutorErr`, the `is_permanent` classification helper, the `backend_err` constructor, and `UploadErr::is_permanent()` in `agent/src/upload/errors.rs`.
- [x] Use `backend_err` for the two backend calls (`create_upload`, `confirm_upload`) in `agent/src/upload/executor.rs`; leave token and transfer paths on `executor_err`.
- [x] Short-circuit permanent errors in `Worker::run_round` in `agent/src/upload/uploader.rs` with a dedicated `log_dropped_permanent` log line.
- [x] Update existing `ExecutorErr` struct literals in `agent/tests/upload/uploader.rs`, `agent/tests/mocks/upload_executor.rs`, `agent/tests/mocks/object_transfer.rs` (add `permanent: false`).
- [x] Add `MockStep::PermanentErr` to `agent/tests/mocks/upload_executor.rs`.
- [x] Add classification unit tests in new file `agent/tests/upload/errors.rs`; register `pub mod errors;` in `agent/tests/upload/mod.rs`.
- [x] Add actor behavior tests (permanent drop after 1 attempt, permanent drop mid-round) in `agent/tests/upload/uploader.rs`.
- [x] Add executor classification tests (404 create → permanent, 500 create → retryable, token failure → retryable, transfer failure → retryable, 404 confirm → permanent) in `agent/tests/upload/executor.rs`.
- [x] Run the test suite from the repo root; all upload-module tests pass (61 passed, 0 failed).
- [ ] Commit, push, and run preflight; CI (Lint + Test/Coverage jobs of `.github/workflows/ci.yml`) green on the pushed branch head.

## Surprises & Discoveries

(Add entries as you go.)

- Observation: `executor_err()` is also used in `agent/src/upload/transfer.rs` with plain `&str` messages (e.g. `executor_err("s3 scheme is missing s3_credentials")`), which do not implement `crate::errors::Error`. This rules out narrowing `executor_err`'s bound to the trait without collateral changes, and reinforced the decision to add a second, trait-bounded constructor instead.
  Evidence: `agent/src/upload/transfer.rs` lines 83, 103, 146.

- Observation: `backend_err()` and `executor_err()` are `pub(crate)` (per the Decision Log), so the integration-test crate under `agent/tests/` cannot call them directly. The constructor-level cases the test plan listed for `agent/tests/upload/errors.rs` (`backend_err` of a 404/500 `RequestFailed`, `executor_err` of an io error) are covered through the real executor path instead: `create_404_is_permanent`, `create_500_is_not_permanent`, and `confirm_404_is_permanent` in `agent/tests/upload/executor.rs` exercise `backend_err` end to end, and the token/transfer failure tests exercise `executor_err`. `agent/tests/upload/errors.rs` tests the public `is_permanent()` helper and `UploadErr::is_permanent()` with literal `ExecutorErr` construction.
  Evidence: integration tests are a separate crate; `pub(crate)` items are invisible to them.

## Decision Log

- Decision: Classify permanence **in the executor, before type erasure**, via a new `backend_err()` constructor applied only to the upload's own backend HTTP calls (`create_upload`, `confirm_upload`) — rather than forwarding `http_status()` / `is_network_conn_err()` through `ExecutorErr` to the boxed source and classifying in the actor.
  Rationale: (1) After `executor_err()` boxes the error, call-site provenance is unrecoverable: an `AuthnErr` from the token manager can wrap an `HTTPErr::RequestFailed` with a 4xx from the *token* endpoint, and a trait-forwarding approach would misclassify it as a permanent failure of the upload job. The task requires token-manager and object-transfer errors to stay retryable. (2) `executor_err()` accepts `&str` sources in `transfer.rs` that do not implement `crate::errors::Error`, so the boxed type cannot be changed to `Box<dyn crate::errors::Error + Send + Sync>` without extra shim types. (3) This mirrors the repo idiom of computing classification where the concrete type is still known (`agent/src/s3/errors.rs` and `agent/src/gcs/errors.rs` map SDK errors to statuses at the boundary; `agent/src/deploy/fsm.rs` consumes the trait only for a simple network/non-network split).
  Date/Author: 2026-07-17 / plan author (Claude).

- Decision: Permanent = `!is_network_conn_err()` AND `http_status().is_client_error()` AND status not in {408 Request Timeout, 429 Too Many Requests, 401 Unauthorized}.
  Rationale: The task mandates 404 → permanent and 408/429 → retryable. 401 is additionally excluded because it signals a stale/expired device token: the `TokenManager` refreshes tokens in the background, so a later attempt can succeed — it is not "clearly a permanent 4xx" for the job itself. All non-HTTP errors report the trait default `http_status()` of 500 and therefore stay retryable by construction. Guardrail from the task: anything not clearly a permanent 4xx HTTP response stays retryable.
  Date/Author: 2026-07-17 / plan author (Claude).

- Decision: Drop-on-permanent is checked in `Worker::run_round` immediately after an attempt fails, before the `max_total_attempts` check, and reuses `Flow::Continue` (the actor stays healthy and proceeds to the next job).
  Rationale: Keeps the retry ladder untouched for transient errors and makes the permanent path a single early exit, following the repo's disqualify-early conditional style.
  Date/Author: 2026-07-17 / plan author (Claude).

- Decision: Plan file placed in `plans/backlog/` (directory created by this plan) rather than `.agents/exec-plans/backlog/`.
  Rationale: This repo's established convention is `plans/` (existing `plans/completed/` with `YYYYMMDD-slug.md` files); the task workflow expects new plans under `plans/backlog/`.
  Date/Author: 2026-07-17 / plan author (Claude).

## Outcomes & Retrospective

(Summarize at completion or major milestones.)

## Context and Orientation

All paths are relative to the repo root (`/home/user/agent` locally). Source lives in `agent/src/`, integration-style tests in `agent/tests/` (mirroring the source module tree), shared test mocks in `agent/tests/mocks/`. Read `AGENTS.md` and `ARCHITECTURE.md` at the repo root for conventions (import ordering with `// standard crates` / `// internal crates` / `// external crates` group comments, error idioms, test commands).

Key terms and files:

- **Upload actor / uploader.** `agent/src/upload/uploader.rs`. A tokio task (`Worker::run`) owning a `Queue` of upload `Job`s. `Worker::run_round` (around line 150) drives up to `UploaderOptions::in_place_attempts` (default 3) executor attempts on one job with backoff sleeps between them (`cooldown::Backoff`, default base 10s, growth 2, max 120s), then requeues the job at the tail; when `entry.attempts` reaches `UploaderOptions::max_total_attempts` (default 9) the job is dropped via `log_dropped`. Today **every** `UploadErr` failure walks this full ladder — that is the bug. An `AttemptOutcome::Failed(err)` carries the `UploadErr` used for logging and, after this change, for the drop decision.

- **Executor.** `agent/src/upload/executor.rs`. Trait `UploadExecutor` with one method `upload(&self, job: &Job)`. Production impl `LiveExecutor`: `create_upload` (POST to the backend uploads endpoint via `http::uploads::create`, wrapped in `http::with_retry`), then `transfer` (cloud SDK put via the `ObjectTransfer` trait in `agent/src/upload/transfer.rs`), then `confirm_upload`, then optional source-file delete. `token()` fetches the device JWT from the `TokenManager` (`agent/src/authn/`).

- **Error erasure — root cause.** `agent/src/upload/errors.rs`. `executor_err(source)` (line ~48) boxes any error into `ExecutorErr { source: Box<dyn std::error::Error + Send + Sync>, trace }` inside `UploadErr::ExecutorErr`. `ExecutorErr` implements the repo error trait with `impl crate::errors::Error for ExecutorErr {}` — all default methods — so `http_status()` reports 500 and `is_network_conn_err()` reports false regardless of the underlying error. The real classification exists on the concrete error: `HTTPErr::RequestFailed` (`agent/src/http/errors.rs`) implements `http_status()` returning the actual response status (e.g. 404), and `HTTPErr::TimeoutErr` / `HTTPErr::ReqwestErr(Connection)` implement `is_network_conn_err() → true`.

- **Error trait.** `agent/src/errors/mod.rs`. `pub trait Error: std::error::Error` with defaulted `code() -> Code`, `http_status() -> HTTPCode` (alias of `axum::http::StatusCode`, same type as `reqwest::StatusCode`), `params()`, `is_network_conn_err()`. Aggregating enums forward via the `crate::impl_error!` macro; `UploadErr` already uses it.

- **Per-request retry (unchanged).** `agent/src/http/retry.rs` `with_retry` retries only `is_network_conn_err()` failures up to 3 total attempts per HTTP request. It already fails 4xx/5xx immediately at the request level; the actor-level ladder above it is what burns the budget.

- **Existing tests.** `agent/tests/upload/uploader.rs` covers the retry ladder using `MockUploadExecutor` (`agent/tests/mocks/upload_executor.rs`) — a scripted mock with `MockStep::{Ok, Err, Hang}` and a `started_rx` channel signaling each `upload` call, plus an injectable `sleep_fn` that records/skips backoff sleeps. `agent/tests/upload/executor.rs` covers `LiveExecutor` with `MockClient` (`agent/tests/mocks/http_client.rs`, per-endpoint closures like `set_create_upload(|| Err(...))`), `StubTokenManager`, and `MockObjectTransfer`. Tests construct real 4xx errors with the idiom used in `agent/tests/services/git_commit/get.rs`:

    HTTPErr::RequestFailed(RequestFailed {
        request: Params::get("http://test/uploads").meta().unwrap(),
        status: reqwest::StatusCode::NOT_FOUND,
        error: None,
        trace: miru_agent::trace!(),
    })

  where `Params` is `miru_agent::http::request::Params`.

- **Coverage gate.** `agent/src/upload/.covgate` requires 97.00% line coverage for the module (enforced by `./scripts/covgate.sh`, which CI runs). New code paths must be exercised by tests.

## Plan of Work

All edits keep the repo's import-group ordering (`// standard crates`, `// internal crates`, `// external crates`) and the 4-space rustfmt style; CI lint enforces both.

**1. `agent/src/upload/errors.rs` — carry and compute the classification.**

Add a `permanent` field to `ExecutorErr`, defaulting to retryable in `executor_err()`. Add a trait-bounded constructor `backend_err()` that computes permanence from the concrete error *before* boxing, a pure public helper `is_permanent()` holding the status rule, and an inherent `UploadErr::is_permanent()` for the actor. Resulting shapes (import `HTTPCode` alongside `Trace` from `crate::errors`):

    #[derive(Debug, thiserror::Error)]
    #[error("upload executor error: {source}")]
    pub struct ExecutorErr {
        #[source]
        pub source: Box<dyn std::error::Error + Send + Sync>,
        /// True only for a definitive client error (4xx excluding 408, 429,
        /// and 401) returned by the upload's own backend request. Permanent
        /// failures are dropped by the actor instead of retried.
        pub permanent: bool,
        pub trace: Box<Trace>,
    }

    impl UploadErr {
        /// Whether retrying this error can never succeed.
        pub fn is_permanent(&self) -> bool {
            matches!(self, Self::ExecutorErr(e) if e.permanent)
        }
    }

    /// Whether `e` is a permanent client error: a definitive 4xx response
    /// that is not a timeout (408), rate limit (429), or stale-token (401)
    /// condition. Network connection errors and non-HTTP errors (which
    /// default to a 500 status) are never permanent.
    pub fn is_permanent<E: crate::errors::Error>(e: &E) -> bool {
        if e.is_network_conn_err() {
            return false;
        }
        let status = e.http_status();
        status.is_client_error()
            && status != HTTPCode::REQUEST_TIMEOUT
            && status != HTTPCode::TOO_MANY_REQUESTS
            && status != HTTPCode::UNAUTHORIZED
    }

    /// Wraps an error from the upload's own backend request, classifying
    /// permanence from the concrete type before erasing it.
    pub(crate) fn backend_err<E>(source: E) -> UploadErr
    where
        E: crate::errors::Error + Send + Sync + 'static,
    {
        let permanent = is_permanent(&source);
        UploadErr::ExecutorErr(ExecutorErr {
            source: Box::new(source),
            permanent,
            trace: crate::trace!(),
        })
    }

`executor_err()` keeps its current signature and behavior, adding `permanent: false` to the struct literal. All existing call sites in `agent/src/upload/transfer.rs` and `LiveExecutor::token()` remain on `executor_err()` and are therefore retryable by construction — this covers the task constraint that token-manager and object-transfer errors must never be misclassified as permanent.

**2. `agent/src/upload/executor.rs` — classify the two backend calls.**

In `create_upload` and `confirm_upload`, change the final `.map_err(executor_err)` to `.map_err(backend_err)` (the error there is the concrete `http::HTTPErr`, which implements `crate::errors::Error` with the real status). Update the import from `crate::upload::errors` to bring in `backend_err`. Do **not** touch `token()` (stays `executor_err`) or anything in `transfer.rs`.

**3. `agent/src/upload/uploader.rs` — drop permanent failures immediately.**

In `Worker::run_round`, immediately after the `AttemptOutcome::Failed(err)` arm yields `err` and before the `max_total_attempts` check, insert:

    if err.is_permanent() {
        Self::log_dropped_permanent(&entry, &err);
        return Flow::Continue;
    }

Add alongside `log_dropped`:

    fn log_dropped_permanent(entry: &QueueEntry, err: &UploadErr) {
        error!(
            "dropping upload job after {} attempt(s) (rule {}, file {}, digest {}): permanent client error, not retrying: {err:?}",
            entry.attempts, entry.job.upload_rule_id, entry.job.file, entry.job.digest
        );
    }

Update the `run_round` doc comment to mention the permanent-error early drop. No changes to `UploaderOptions`, the backoff math, requeue, or shutdown paths.

**4. Test mocks — new struct field and permanent step.**

`ExecutorErr` gains a field, so every existing struct literal must add `permanent: false`. Grep-verified complete list of construction sites outside `agent/src/upload/errors.rs`:

- `agent/tests/upload/uploader.rs` — `scripted_err()`.
- `agent/tests/mocks/upload_executor.rs` — the `MockStep::Err` arm.
- `agent/tests/mocks/object_transfer.rs` — the scripted error push.

In `agent/tests/mocks/upload_executor.rs`, extend the script enum and the `upload` match:

    pub enum MockStep {
        Ok,
        Err,
        /// A permanent (non-retryable) executor error.
        PermanentErr,
        Hang(oneshot::Receiver<Result<(), UploadErr>>),
    }

    Some(MockStep::PermanentErr) => Err(UploadErr::ExecutorErr(ExecutorErr {
        source: Box::new(std::io::Error::other("scripted permanent failure")),
        permanent: true,
        trace: miru_agent::trace!(),
    })),

**5. Classification unit tests — new file `agent/tests/upload/errors.rs`.**

Register `pub mod errors;` in `agent/tests/upload/mod.rs` (keep alphabetical order: `errors`, `executor`, `queue`, `transfer`, `uploader`). Test the helper and the enum method directly. Build `RequestFailed` values with the `Params::get("http://test/uploads").meta().unwrap()` idiom (Context section above). Required cases:

- `is_permanent` on `HTTPErr::RequestFailed` with status 404 → true (the incident case); with 400 → true.
- Status 408 → false; 429 → false; 401 → false.
- Status 500 → false; 503 → false.
- Network connection error → false: use `HTTPErr::MockErr(http MockErr { is_network_conn_err: true })` from `miru_agent::http::errors`.
- Non-HTTP error → false: any error type using the trait defaults (e.g. `upload::errors::QueueFullErr`), proving the 500 default keeps it retryable.
- `UploadErr::is_permanent()`: true for an `ExecutorErr` with `permanent: true`; false for `executor_err(std::io::Error::other(...))`, for `backend_err` of a 500 `RequestFailed`, and for the `QueueFullErr` / `SendActorMessageErr` variants.
- `backend_err` of a 404 `RequestFailed` → `UploadErr::is_permanent()` is true (constructor and method agree end to end).

Avoid 4+ consecutive `assert_eq!` on fields of one variable per test function (the repo's field-by-field-assert lint); prefer one boolean assert per case or a table-driven loop.

**6. Actor behavior tests — extend `agent/tests/upload/uploader.rs`.**

Follow the file's existing idioms (`timed`, `make_job`, `spawn_uploader`, `started_rx` sequencing, recorded sleep vectors — see `retry_backoff_follows_expected_sequence` for the sleep-recording pattern).

- `permanent_error_drops_job_after_one_attempt`: script `MockStep::PermanentErr` then `MockStep::Ok`. Use a sleep-recording `sleep_fn`. Enqueue job A, await one `started_rx`; enqueue job B, await one `started_rx`. Assert `mock.recorded_calls() == vec![job_a, job_b]` (A attempted exactly once — no in-place retries, no requeue), the recorded sleeps vector is empty (no backoff), and `uploader.len().await == 0`. Shut down and join, as the sibling tests do. This test fails before the source change (A would be attempted 3+ times) and passes after.
- `permanent_error_mid_round_drops_immediately`: script `MockStep::Err`, `MockStep::PermanentErr`, `MockStep::Ok`. Enqueue A then B. Assert calls are `[A, A, B]` (transient failure, one backoff sleep recorded, then the permanent failure ends the job inside the round) and the sleeps vector has exactly 1 entry.
- Transient regression guard: the existing `global_attempt_cap_drops_job` (9 attempts then drop) and `retry_backoff_follows_expected_sequence` tests must pass **unmodified** — they are the proof that transient behavior is byte-for-byte unchanged.

**7. Executor classification tests — extend `agent/tests/upload/executor.rs`.**

Using the existing `MockClient` / `StubTokenManager` / `MockObjectTransfer` harness (see `create_failure_maps_to_executor_err` for the pattern):

- `create_404_is_permanent`: `client.set_create_upload(|| Err(HTTPErr::RequestFailed(... status NOT_FOUND ...)))`; `executor.upload(&job)` errs and `err.is_permanent()` is true.
- `create_500_is_not_permanent`: same with `INTERNAL_SERVER_ERROR`; `err.is_permanent()` is false.
- `confirm_404_is_permanent`: happy create + transfer, `client.set_confirm_upload(|| Err(... NOT_FOUND ...))`; `err.is_permanent()` is true.
- `token_failure_is_not_permanent`: `StubTokenManager::err(AuthnErr::MockError(...))`; `err.is_permanent()` is false (extend the existing `token_failure_maps_to_executor_err` test with this assert rather than duplicating it, if cleaner).
- `transfer_failure_is_not_permanent`: scripted `MockObjectTransfer` error; `err.is_permanent()` is false.

## Concrete Steps

All commands run from the repo root, `/home/user/agent`, on branch `claude/task-mode-pr-agent-nxwei3`.

1. Make the source edits of Plan of Work steps 1–3, then the test edits of steps 4–7, in that order (source first so the new field's compile errors point at every literal that needs `permanent: false`).

2. Build and run the full suite (the `--features test` flag is mandatory; the script sets it):

       ./scripts/test.sh

   Expected: compiles cleanly; all tests pass, including the 2 new uploader tests, the new `upload::errors` classification tests, and the ~5 new/extended executor tests. Expected transcript tail (counts will be higher; zero failures is the requirement):

       test result: ok. ... passed; 0 failed; ... filtered out

3. Optional local lint sanity check (CI is authoritative):

       ./scripts/update-deps.sh
       LINT_FIX=0 ./scripts/lint.sh

   Expected: no import-order, fmt, clippy, or field-by-field-assert findings in the touched files.

4. Commit the changes from the repo root with a Conventional Commit, e.g.:

       git add agent/src/upload agent/tests plans/
       git commit -m "fix: drop upload jobs on permanent 4xx backend errors instead of retrying"

5. Push the branch and run the `$preflight` workflow (agent dispatch): push to `origin claude/task-mode-pr-agent-nxwei3`, watch the CI run on the pushed head (`.github/workflows/ci.yml`: Lint job running `LINT_FIX=0 ./scripts/lint.sh`, Test job running `./scripts/covgate.sh`, plus the tools lint/covgate job), and fix any failures from the CI logs, iterating until green.

6. Repeatable safely: steps 2–3 are read-only checks; step 5's push/watch loop is idempotent per commit.

## Validation and Acceptance

Acceptance is behavioral:

1. From `/home/user/agent`, run `./scripts/test.sh`: all tests pass. Before the source change, the new test `permanent_error_drops_job_after_one_attempt` in `agent/tests/upload/uploader.rs` fails (the mock records 3+ attempts for job A); after the change it passes (exactly 1 attempt, empty sleep vector). The pre-existing `global_attempt_cap_drops_job` and `retry_backoff_follows_expected_sequence` tests pass unmodified, proving transient-error behavior (9 total attempts, 3-per-round backoff/requeue ladder) is unchanged.
2. Classification unit tests in `agent/tests/upload/errors.rs` pass: 404 and 400 classify permanent; 408, 429, 401, 500, 503, network-connection errors, and non-HTTP errors classify retryable.
3. Executor tests prove the boundary: a 404 from `create_upload` or `confirm_upload` surfaces as `UploadErr::is_permanent() == true`; token-manager and object-transfer failures surface as retryable.
4. `./scripts/covgate.sh` (run by CI) passes the `agent/src/upload/.covgate` threshold of 97.00%.
5. **Preflight gate (mandatory): preflight must report CLEAN — the CI workflow (`.github/workflows/ci.yml`) green on the pushed head of `claude/task-mode-pr-agent-nxwei3` — before the PR leaves draft or the task is reported complete.** No local result substitutes for the CI signal.

Runtime acceptance (for reviewers reading logs after deployment): a device whose upload rule was deleted logs a single `dropping upload job after 1 attempt(s) (rule upl_col_..., file ..., digest ...): permanent client error, not retrying: ...` line per job instead of nine retry attempts over ~5 minutes.

## Idempotence and Recovery

- All edits are plain-text source changes on a dedicated branch; re-running any step is safe. If a step fails midway, fix and re-run `./scripts/test.sh` — nothing is stateful.
- The compiler enforces completeness of the `ExecutorErr` field addition: any missed `permanent:` literal is a build error, not a silent bug.
- If CI reveals a coverage-gate failure on `agent/src/upload`, add tests for the uncovered lines (most likely a branch of `is_permanent` or the `log_dropped_permanent` path) rather than lowering `.covgate`.
- Rollback: `git revert` the commit(s) on the branch; no data, schema, or config migration is involved.
- If the classification proves too aggressive in the field, the single knob is `is_permanent()` in `agent/src/upload/errors.rs` — narrowing it to an allowlist (e.g. only 404) is a one-line change with its unit tests adjusted accordingly.
