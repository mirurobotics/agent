# Treat 4xx create-upload rejections as terminal: drop the upload job instead of requeueing it

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (repo root of mirurobotics/agent) | read-write | Add an `is_terminal()` error classification to the shared error trait, propagate it from the backend HTTP layer through the upload executor's error wrapper, and make the uploader actor drop (instead of requeue) a job whose failure is terminal — with a loud structured log carrying the HTTP status. Changes touch `agent/src/errors/mod.rs`, `agent/src/http/errors.rs`, `agent/src/upload/errors.rs`, `agent/src/upload/executor.rs`, `agent/src/upload/uploader.rs`, and mirror test files under `agent/tests/`. |
| `libs/backend-api/` | read-only | Generated OpenAPI client code; the create-upload request/response models are already correct. Not modified. |

This plan lives in the agent repo because every code change is inside `agent/`. Work happens on branch `fix/upload-create-4xx-terminal` (already checked out, clean, based on `main`). All commands run from the repo root.

## Purpose / Big Picture

The uploader actor drains a queue of upload jobs. Each attempt calls the backend's create-upload endpoint (`POST /uploads`). Today, when the backend rejects that call with a 4xx (for example the upload rule was deleted, or the request is permanently invalid), the job is treated exactly like a transient failure: it burns 3 in-place attempts with backoff, is requeued at the tail, and repeats until 9 total attempts — roughly nine futile round-trips before a quiet drop.

After this change, a 4xx rejection that cannot succeed on retry kills the job immediately: one attempt, one `error!` log that names the HTTP status and the job (rule, file, digest), no requeue. Carve-outs that stay retryable exactly as today: **401** (unauthorized — the device token may refresh), **408** (request timeout), **429** (rate limited), every 5xx, and network/timeout errors. The loud log matters because there is no backend failure-reporting endpoint — this log line is the only record that the upload died.

Observable outcome: run the test suite and see the new uploader test prove a terminal error produces exactly one executor call and no requeue, while the existing requeue/attempt-cap tests still pass unchanged.

## Progress

- [x] Milestone 1: `is_terminal()` on the `Error` trait + `impl_error!` + `RequestFailed` override; classification tests. (commits c21adb7, f9ecf29)
- [x] Milestone 2: propagate through `ExecutorErr`; uploader drops terminal jobs with the loud log; executor + uploader tests. (commits b004d5a, f9ecf29)
- [ ] Milestone 3: preflight to CI-green on the pushed branch head.

## Surprises & Discoveries

- Coverage for `impl_error!`-generated `is_terminal()` bodies attributes to the macro definition site (`agent/src/errors/mod.rs`), not to each invoking module. Measured at the milestone-2 HEAD, only the errors/http/upload gates dipped; the plan's contingency for unrelated-module covgate dips was never needed. A single dispatch test through `HTTPErr::MockErr` restored `errors` to 100.
- The `_ => None` wildcard arm of `UploadErr::terminal_status()` was unreachable by the planned tests; a one-line `terminal_status() == None` assertion in the existing `enqueue_after_shutdown_returns_send_err` test covers it (actor-channel errors are never terminal) and keeps the tight 96.00 upload gate green (96.81 measured).

## Decision Log

(Add entries as work proceeds. Pre-authoring decisions that shaped this plan:)

- Decision: classify terminality on the error type via a trait method (`is_terminal()`), mirroring `is_network_conn_err()`.
  Rationale: `deploy/fsm.rs` (`should_bump_attempts`, ~line 197) and `sync/syncer.rs` (`handle_sync_failure`, ~line 219) already branch on `is_network_conn_err()`; a sibling trait method is the established pattern. Date/Author: 2026-07-17 / ben@miruml.com.
- Decision: carry the classification across the type-erased executor boundary as a `terminal_status: Option<HTTPCode>` field captured at wrap time, not by changing `ExecutorErr.source` to a `Box<dyn crate::errors::Error>` trait object.
  Rationale: `ExecutorErr.source` is `Box<dyn std::error::Error + Send + Sync>`, which erases the custom trait, and three `transfer.rs` call sites pass plain `&str` messages that do not implement `crate::errors::Error`. Capturing a scalar at wrap time keeps `executor_err`'s bound (and the string call sites) untouched and avoids thiserror `#[source]`/trait-upcasting subtleties. Date/Author: 2026-07-17 / ben@miruml.com.
- Decision: do NOT override `http_status()` on `ExecutorErr`.
  Rationale: `ServerErr` wraps `UploadErr` (`agent/src/server/errors.rs` line 118) and server handlers map `e.http_status()` straight into device-API responses (`agent/src/server/handlers.rs` line 160). Overriding it could silently change response codes if an `ExecutorErr` ever surfaces there. The dedicated `terminal_status` field serves classification and logging only. Date/Author: 2026-07-17 / ben@miruml.com.
- Decision: apply the classified wrapper to both backend calls the executor makes — `create_upload` (required by the task) and `confirm_upload` (natural extension).
  Rationale: both wrap `http::with_retry` and map through the same `executor_err`; excluding confirm would need an artificial second error path. A terminal 4xx on confirm is equally unrecoverable by retrying. Note there is no separate vend-credentials call in the executor: credentials arrive embedded in the create response (`UploadWithCredentials`); the executor calls only `create` and `confirm`, and the `vend_credentials` helper in `agent/src/http/uploads.rs` has no production callers. The `token()` step keeps the unclassified `executor_err` so authentication errors can never be classified terminal (preserves the 401/token-refresh carve-out end to end). Date/Author: 2026-07-17 / ben@miruml.com.
- Decision: drop = "do not requeue". No extra cleanup runs.
  Rationale: identical to the existing `max_total_attempts` drop path — the entry simply is not pushed back, and the queue snapshot excludes it on the next persist. `Job` carries no other persisted state (confirmed by `plans/completed/20260717-store-upload-metadata.md`: create-response metadata lives only inside a single `upload()` call). The source file is left on disk, same as today's drop path. Date/Author: 2026-07-17 / ben@miruml.com.

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

Repo root: the checkout of `mirurobotics/agent` (a Rust workspace; the binary crate is `miru-agent` under `agent/`). Paths are repo-relative; commands run from the repo root. Read `AGENTS.md` first — import ordering (standard/internal/external groups), error conventions, and `./scripts/test.sh` usage all apply.

How an HTTP 4xx currently travels to the uploader:

1. `agent/src/http/response.rs` turns a non-2xx response into `HTTPErr::RequestFailed(RequestFailed)` (`agent/src/http/errors.rs` lines 6-12). `RequestFailed` carries `status: reqwest::StatusCode` and `request: request::Meta` (url, method, timeout — `agent/src/http/request.rs` line 115). Its `crate::errors::Error` impl (lines 28-46) overrides `http_status()` to return that status; `is_network_conn_err()` stays the default `false`.
2. `agent/src/http/retry.rs::with_retry` (lines 26-47) retries only when `e.is_network_conn_err()` is true (up to 2 retries). Any HTTP status failure — 4xx or 5xx — already exits the retry loop immediately. **No change is needed here.**
3. `agent/src/upload/executor.rs::LiveExecutor` does `create_upload` (lines 53-65, wraps `http::uploads::create` in `with_retry`), then the object transfer, then `confirm_upload` (lines 67-79). Both HTTP steps map errors with `.map_err(executor_err)`.
4. `agent/src/upload/errors.rs::executor_err` (lines 48-56) boxes any error into `UploadErr::ExecutorErr(ExecutorErr { source: Box<dyn std::error::Error + Send + Sync>, trace })`. This **erases** the custom trait: `impl crate::errors::Error for ExecutorErr {}` (line 25) is all defaults, so the status code is invisible to the uploader.
5. `agent/src/upload/uploader.rs::Worker::run_round` (lines 154-196) drives up to `in_place_attempts` (default 3) attempts with backoff; on a round-ending failure it requeues the entry at the tail; at `max_total_attempts` (default 9, `UploaderOptions` lines 31-59) total failures it drops the job via `log_dropped` (lines 251-256, an `error!` with rule/file/digest). `QueueEntry` is `{ job: Job, attempts: u32 }`; `Job` (`agent/src/upload/job.rs`) has `file`, `size`, `digest`, `upload_rule_id`, `deployment_id`, etc. for log context.

Terms: **terminal error** (introduced by this plan) = a failure that retrying cannot fix — a backend 4xx response other than 401/408/429. **`HTTPCode`** = `axum::http::StatusCode`, aliased in `agent/src/errors/mod.rs` line 6; it is the same underlying `http::StatusCode` type as `reqwest::StatusCode` (which is why `RequestFailed::http_status` returns `self.status` unconverted).

The shared error machinery lives in `agent/src/errors/mod.rs`: the `Error` trait (lines 29-42) with defaulted `code()`, `http_status()`, `params()`, `is_network_conn_err()`, and the `impl_error!` macro (lines 63-88) that generates a delegating impl for every aggregating error enum in the repo (`HTTPErr`, `UploadErr`, `ServerErr`, ...). Adding a method to the trait plus a dispatch arm to the macro propagates it everywhere with no per-enum edits.

Tests mirror `agent/src/` under `agent/tests/`. Relevant rigs:

- `agent/tests/http/errors.rs` — http error unit tests (classification test goes here).
- `agent/tests/upload/executor.rs` — builds a `LiveExecutor<MockClient, MockTokenManager, MockObjectTransfer>`; `MockClient` (`agent/tests/mocks/http_client.rs`) scripts each endpoint via closures, e.g. `create_upload_fn: Mutex<Box<dyn Fn() -> Result<UploadWithCredentials, HTTPErr> + Send + Sync>>`, so a test can make create return any `HTTPErr`.
- `agent/tests/upload/uploader.rs` + `agent/tests/mocks/upload_executor.rs` — the actor tests script `MockStep::{Ok, Err, Hang}` per `upload()` call and assert on `recorded_calls()`; a `started_rx` channel sequences attempts deterministically.

Coverage gates (`scripts/covgate.sh`): `agent/src/upload/.covgate` = 96.00, `agent/src/http/.covgate` = 93.9, `agent/src/errors/.covgate` = 100.

## Plan of Work

Milestone 1 — classification primitive.

1. `agent/src/errors/mod.rs`: add to the `Error` trait, after `is_network_conn_err`:

       /// True when retrying can never succeed (e.g. the backend rejected the
       /// request with a non-transient 4xx). Default: false.
       fn is_terminal(&self) -> bool {
           false
       }

   Add the matching dispatch arm inside `impl_error!` (same shape as the `is_network_conn_err` arm):

       fn is_terminal(&self) -> bool {
           match self {
               $(Self::$variant(e) => e.is_terminal(),)+
           }
       }

2. `agent/src/http/errors.rs`: override on `RequestFailed`'s existing `crate::errors::Error` impl:

       fn is_terminal(&self) -> bool {
           self.status.is_client_error() && !matches!(self.status.as_u16(), 401 | 408 | 429)
       }

   Every other error type keeps the default `false` — network errors, timeouts, decode errors, and all 5xx remain non-terminal.

3. Tests in `agent/tests/http/errors.rs`: a table-driven test constructing `RequestFailed` (helper building `request::Meta { url, method, timeout }`, `error: None`, `trace: miru_agent::trace!()`) and asserting `is_terminal()` per status: true for 400, 403, 404, 409, 422; false for 401, 408, 429, 500, 502, 503. Also assert one non-status error (e.g. `HTTPErr::MockErr`) defaults to false through the enum dispatch. Import `miru_agent::errors::Error` to call the trait method. A loop over `(u16, bool)` tuples keeps it lean and avoids the field-by-field-assert lint.

Milestone 2 — propagation and the drop path.

4. `agent/src/upload/errors.rs`:
   - Add `terminal_status: Option<HTTPCode>` to `ExecutorErr` (import `HTTPCode` from `crate::errors`). Doc comment: "Some(status) when the wrapped failure was classified terminal at wrap time."
   - Override on its impl: `fn is_terminal(&self) -> bool { self.terminal_status.is_some() }`.
   - `executor_err` (bound unchanged) sets `terminal_status: None`.
   - New sibling used where the concrete error type still implements the trait:

         /// Like [`executor_err`], but captures the terminal classification (and
         /// the backend status behind it) before the type is erased.
         pub(crate) fn classified_executor_err<E>(source: E) -> UploadErr
         where
             E: crate::errors::Error + Send + Sync + 'static,
         {
             let terminal_status = source.is_terminal().then_some(source.http_status());
             UploadErr::ExecutorErr(ExecutorErr {
                 source: Box::new(source),
                 terminal_status,
                 trace: crate::trace!(),
             })
         }

   - Inherent accessor for the uploader's log:

         impl UploadErr {
             /// The backend HTTP status behind a terminal failure, if any.
             pub fn terminal_status(&self) -> Option<HTTPCode> {
                 match self {
                     Self::ExecutorErr(e) => e.terminal_status,
                     _ => None,
                 }
             }
         }

5. `agent/src/upload/executor.rs`: in `create_upload` and `confirm_upload`, change `.map_err(executor_err)` to `.map_err(classified_executor_err)` (update the `use crate::upload::errors::...` import). `token()` and everything in `transfer.rs` keep `executor_err`.

6. `agent/src/upload/uploader.rs`, `run_round`: immediately after the `AttemptOutcome::Failed(err)` arm yields `err` (before the `max_total_attempts` check), add:

       if let Some(status) = err.terminal_status() {
           Self::log_terminal_drop(&entry, status, &err);
           return Flow::Continue;
       }

   and next to `log_dropped`:

       fn log_terminal_drop(entry: &QueueEntry, status: HTTPCode, err: &UploadErr) {
           error!(
               "dropping upload job: backend rejected it with terminal HTTP status {status} \
                (rule {}, file {}, digest {}, attempt {}); the backend will not learn this \
                upload died: {err:?}",
               entry.job.upload_rule_id, entry.job.file, entry.job.digest, entry.attempts
           );
       }

   Import `HTTPCode` from `crate::errors` (internal-crates group). Returning `Flow::Continue` without requeueing is the drop — identical mechanics to the attempt-cap drop; the entry falls out of scope and the snapshot excludes it on the next queue persist.

7. Test-side updates (all three literal `ExecutorErr { ... }` constructions gain `terminal_status: None`): `agent/tests/upload/uploader.rs` (`scripted_err`, ~line 42), `agent/tests/mocks/upload_executor.rs` (~line 62), `agent/tests/mocks/object_transfer.rs` (~line 43). Add a `MockStep::TerminalErr` variant to `agent/tests/mocks/upload_executor.rs` returning an `ExecutorErr` with `terminal_status: Some(StatusCode::BAD_REQUEST)` (any 4xx works; source can stay the scripted `std::io::Error`).

8. New tests:
   - `agent/tests/upload/executor.rs`: a helper `request_failed_err(status: u16) -> HTTPErr`; a test scripting `create_upload_fn` to return it with 404 and asserting the `upload()` error has `terminal_status() == Some(StatusCode::NOT_FOUND)` (and `is_terminal()` true via the trait); a table-driven companion asserting 401, 408, 429, and 500 yield `terminal_status() == None`; one test scripting `confirm_upload_fn` with a 4xx (create and transfer succeed) asserting the confirm-side error is terminal too.
   - `agent/tests/upload/uploader.rs`: `terminal_failure_drops_job_without_requeue` — push `MockStep::TerminalErr` then `MockStep::Ok`; enqueue job A then job B; drive via `started_rx`; assert `recorded_calls() == vec![job_a, job_b]` (job A attempted exactly once — no in-place retries, no requeue) and `uploader.len()` is 0; job B succeeding proves the worker kept running. Existing tests (`failing_round_requeues_at_tail_behind_later_job`, `global_attempt_cap_drops_job`, `retry_backoff_follows_expected_sequence`) must pass unchanged — they are the proof that non-terminal behavior is untouched.

Milestone 3 — preflight to CI-green (see Validation and Acceptance).

Out of scope: `with_retry` (already correct), `UploaderOptions` (no new knobs), the deploy/sync subsystems (they may adopt `is_terminal()` later; this plan only adds the primitive), generated `libs/` code, and any OpenAPI spec change.

## Concrete Steps

All commands run from the repo root on branch `fix/upload-create-4xx-terminal`.

Milestone 1:

1. Edit `agent/src/errors/mod.rs` and `agent/src/http/errors.rs` per Plan of Work items 1-2.
2. Add the classification test to `agent/tests/http/errors.rs` (item 3).
3. Run:

       ./scripts/test.sh

   Expect: all tests pass, including the new classification test.
4. Commit (from the repo root):

       git add -A && git commit -m "feat(errors): add is_terminal() classification for non-retryable HTTP failures"

Milestone 2:

5. Edit `agent/src/upload/errors.rs`, `agent/src/upload/executor.rs`, `agent/src/upload/uploader.rs` per items 4-6.
6. Update the three test-side `ExecutorErr` literals, add `MockStep::TerminalErr`, and add the executor/uploader tests per items 7-8. `cargo check --tests --features test --package miru-agent` after the struct change surfaces every literal the compiler still wants fixed.
7. Run:

       ./scripts/test.sh
       ./scripts/covgate.sh

   Expect: all tests pass; all coverage gates green (upload ≥ 96.00, http ≥ 93.9, errors ≥ 100 — the new code is exercised by the new tests).
8. Run the full local lint pass:

       ./scripts/update-deps.sh && ./scripts/lint.sh

   Expect: clean (fmt, import linter, clippy `-D warnings`, machete, audit).
9. Commit:

       git add -A && git commit -m "feat(upload): drop jobs immediately on terminal 4xx backend rejections"

Milestone 3:

10. Run the repo preflight (runs lint, covgate, and the tools self-lint/covgate in parallel):

        ./scripts/preflight.sh

    Expected tail of output:

        Preflight clean

11. Push the branch and open a **draft** PR against `main`. Watch the Lint and Test workflows on the pushed head; fix any failure and repeat. Commit any fixes as their own commit.

## Validation and Acceptance

- `./scripts/test.sh` (repo root) passes with zero failures.
- The new uploader test `terminal_failure_drops_job_without_requeue` **fails before** the Milestone-2 uploader change (a terminal error still triggers 3 in-place attempts and a requeue, so `recorded_calls()` shows extra attempts) and **passes after** it.
- The classification test proves: 400/403/404/409/422 → terminal; 401/408/429 and all 5xx → not terminal; network/timeout/decode errors → not terminal.
- Executor tests prove a 4xx from create (and from confirm) reaches the uploader with `terminal_status() == Some(status)`, while 401/408/429/500 arrive with `None`.
- Existing requeue/attempt-cap/backoff uploader tests pass unchanged (non-terminal behavior is untouched).
- The terminal drop emits a single `error!` line containing the literal HTTP status and the job's rule id, file path, digest, and attempt count (verify by reading `log_terminal_drop`; tests run with `RUST_LOG=off`, so the log is asserted by review, not capture).
- `./scripts/covgate.sh` — every gate green.
- **Release gate: preflight must report CLEAN — `./scripts/preflight.sh` prints `Preflight clean` locally AND CI (Lint + Test workflows) is green on the pushed head of `fix/upload-create-4xx-terminal` — before the PR leaves draft or the task is reported complete.**

## Idempotence and Recovery

Every step is an idempotent file edit plus a re-runnable script; `test.sh`, `covgate.sh`, `lint.sh`, and `preflight.sh` are safe to repeat. No migrations, no destructive steps. If a milestone goes sideways, `git checkout -- <file>` (or `git reset --hard` to the last milestone commit) restores a known-good state; the two milestone commits keep the branch bisectable.

Known risk: adding the `is_terminal` arm to `impl_error!` generates one new method body per aggregating enum across the repo, and macro-expanded lines attribute to each invocation site's module. If an unrelated module's covgate dips because its generated `is_terminal()` is never called, prefer adding a one-line `is_terminal()` assertion to that module's existing error test; only if that is unreasonable, adjust the gate via `./scripts/update-covgates.sh` and record it in the Decision Log.
