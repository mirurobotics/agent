# Preserve network-error classification in the upload pipeline and exempt network failures from attempt accounting

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (this repo, `repos/agent` in the workbench) | read-write | Upload error types, uploader actor retry policy, tests |

This plan lives in `agent/plans/` because all code changes are in this repository. Work happens on branch `fix/upload-network-error-attempts` (base: `main`).

## Purpose / Big Picture

Today the upload actor counts a network drop the same as a real failure: every failed attempt burns one unit of the per-job attempt budget (`max_total_attempts: 9`), and when the budget is exhausted the job is dropped forever — the scanner's ledger already marked the file as reported, so nothing re-drives it. A network outage of a few minutes is enough to permanently lose every job that cycles through the queue.

After this change, the uploader treats network-classified failures the way the deploy FSM (`agent/src/deploy/fsm.rs`), the syncer (`agent/src/sync/syncer.rs`), and the MQTT worker (`agent/src/workers/mqtt.rs`) already treat them: they do not consume the attempt budget, they trigger a flat base cooldown instead of exponential backoff, and they log at debug level. Observable outcome: with the network down, a queued upload job survives indefinitely (attempts never reach the drop threshold) and completes as soon as connectivity returns; a genuine failure (e.g. HTTP 5xx, malformed credentials) still burns attempts and is dropped at the cap with an error log, exactly as before.

## Progress

- [x] Milestone 1: classification propagation across the executor boundary (`upload/errors.rs`, `upload/executor.rs`, `upload/transfer.rs`) + unit tests.
- [x] Milestone 2: uploader retry policy (no attempt bump, flat cooldown, tail requeue, debug logging) + actor tests.
- [x] Milestone 3: post-v0.9.0 audit confirmation (PR #178 audited post-merge, see Decision Log), local validation of the upload test suite, push, CI-clean preflight.

## Surprises & Discoveries

- PR #178 (upload attempt-deadline) merged to `main` as 6749a4c while this plan was in flight — the exact contingency the Milestone 3 audit anticipated. The branch rebased onto it with zero conflicts: #178 touches `attempt_upload`, `UploaderOptions`, and the `errors` imports, while this plan touches `run_round` and the `ExecutorErr` type, so the hunks are disjoint. All 71 upload tests (including #178's paused-clock `hung_attempt_times_out_and_is_retried`) pass on the combined tree.
- A third `ExecutorErr` struct literal exists in `agent/tests/mocks/object_transfer.rs` (`push_err`), beyond the two the plan listed; it gained the new field like the others.
- The `network_failure_requeues_at_tail` test cannot use `MockStep::NetworkErr` directly — the failure must be released *after* job B is enqueued, so it uses `MockStep::Hang` released with a new `scripted_network_err()` helper. Expected call order `[A, B, A]` also proves the round ends after one attempt.
- The original test plan left `await_network_cooldown`'s shutdown arm uncovered (a covgate liability); `shutdown_during_network_cooldown_returns_promptly` was added to cover it.
- PR #186 (single attempt per pop with deferred per-job backoff) and PR #183 (terminal classification as a boolean flag) both merged to `main` while this branch sat unmerged, and between them they deleted the architecture this plan was written against: `run_round`, `in_place_attempts`, `max_total_attempts`, and `terminal_status()` are all gone. The intent survived the rebase but the implementation was rewritten — see the Decision Log.

## Decision Log

- Decision: capture `is_network_conn_err: bool` on `ExecutorErr` at wrap time (Option B) rather than boxing the source as `Box<dyn crate::errors::Error + Send + Sync>` and delegating (Option A).
  Rationale: (1) the exact precedent already exists in this very type — PR #179 added `terminal_status: Option<HTTPCode>` captured at wrap time by `classified_executor_err()`, and `sync/syncer.rs` uses the same pattern (`SyncFailure { is_network_conn_err }`); (2) Option A breaks the string-message call sites in `upload/transfer.rs` (`executor_err("s3 scheme is missing s3_credentials")` etc.) because `&str` does not implement `crate::errors::Error`, forcing a new concrete error type or dual constructors; (3) capture keeps the field a plain `Box<dyn std::error::Error + Send + Sync>`, so the thiserror `#[source]` chain and Debug formatting are untouched and no trait upcasting is involved; (4) no consumer of `UploadErr` needs `code()`/`http_status()`/`params()` delegation today — the actor consults only `terminal_status()` and (after this change) `is_network_conn_err()`. If a consumer ever needs the rest, Option A can be revisited then.
  Date/Author: 2026-07-20 / plan author.
- Decision: on a network-classified failure the job is requeued at the tail immediately (ending the round), then the actor sleeps the flat cooldown before popping the next job.
  Rationale: the executor path has two independent network destinations — create/confirm go to the Miru backend, the transfer goes to S3 or GCS. A partial outage (e.g. GCS unreachable, backend fine) would head-of-line block distinct-destination jobs if the failed job stayed in place. Tail-requeue rotates the queue so every job gets a shot per cooldown period; during a total outage rotation is harmless (the flat sleep bounds the global attempt rate to one per `base_secs` either way). Requeue happens *before* the sleep so a queue snapshot or shutdown during the sleep never loses the job.
  Date/Author: 2026-07-20 / plan author.
- Decision: PR #178's `AttemptTimeoutErr` needs no classification change. It is a bare expiry (file/size/deadline only, no captured source), and its `crate::errors::Error` impl uses the trait defaults, so `is_network_conn_err() == false` — a deadline expiry burns the attempt budget as a real failure, exactly as the audit guidance requires. Under the reworked `run_round` it rides the non-network path (bump, warn, exponential in-place backoff, cap drop) unchanged.
  Date/Author: 2026-07-20 / implementation.
- Decision: a refine-pass attempt to demote the pre-existing requeue `info!` log to `debug!` on network rounds was reverted — `Queue::requeue` logs `info!` unconditionally right after, so the demotion was half-effective and completing it would churn `queue.rs` out of scope. Accepted residual: pre-outcome `info!` lines ("dequeued job", "attempting", queue-side logs) still fire each network cycle; the failure itself and the cooldown log are `debug!` as specified.
  Date/Author: 2026-07-20 / implementation.
- Decision: on the rebase onto PR #186, re-express the policy in the new architecture rather than restore the round loop. There is no in-place loop and no sleeping in `run_attempt` any more, so: a network failure skips the `entry.attempts` bump, stamps `entry.next_attempt_at` with the flat `base_secs` instead of `cooldown::calc(...)`, and requeues at the tail — which #186 already made the universal path. `await_network_cooldown` was deleted: the waiting now happens in the run loop's central `idle_wait`, whose shutdown responsiveness is already covered by `shutdown_during_backoff_sleep_returns_promptly` and `worker_exits_when_handles_dropped_during_idle_wait`, so `shutdown_during_network_cooldown_returns_promptly` was dropped as duplicate coverage rather than rewritten.
  Rationale: the plan's intent is an accounting and cooldown policy ("a network drop must not burn the attempt budget, and must not back off exponentially"), not the loop shape that happened to express it. Both properties are directly expressible in #186's model, and the remaining three actor tests pin them.
  Date/Author: 2026-08-12 / rebase.
- Decision: bound the exemption with `UploaderOptions::max_job_age` (7 days, measured from `job.first_observed_at`) rather than ship an unbounded exemption.
  Rationale: review raised the misclassification case the plan never addressed. Classification is opt-in (`is_network_conn_err()` defaults false) and keyed on structural SDK signals — `SdkError::TimeoutError`/`DispatchFailure`, `ReqwestErrKind::Connection`, GCS's `is_connect`/`is_io`/`is_transport()` with no HTTP status — so a systematic false positive is unlikely; but the consequence of one is unbounded, not merely wasteful. Exempt from attempt accounting and never terminal, a misclassified permanent failure retries forever: it holds one of 1024 queue slots for the life of the device and blocks the retention delete that would reclaim its source file, so enough of them turn into `QueueFullErr` on every new enqueue plus a full disk. The GCS predicates are the likeliest source — `is_io()` is broad and they are `doc(hidden)`, so an SDK bump can widen them silently. A wall-clock bound degrades that from "never" to "dropped in bounded time" without weakening the outage behavior the plan exists for: with `attempts: 30` and an hour-capped backoff a normal job dies in ~30h, so 7 days of nothing but network failures is already outside any plausible outage.
  Date/Author: 2026-08-12 / review.

## Outcomes & Retrospective

Implemented as planned across three commits, then rebased twice — onto PR #178 cleanly, and onto PRs #183/#186 with a rewrite (see the Decision Log). As merged: `ExecutorErr` captures `is_network_conn_err` at wrap time and the four erasing call sites (executor `token()`, S3 put, GCS store build, GCS put) go through `classified_executor_err`; `run_attempt` bumps `entry.attempts` only after the outcome is known and only for non-network failures, and a network failure logs at debug, stamps a flat `base_secs` next-attempt deadline instead of the growing backoff, and requeues at the tail. Network failures can never reach the `attempts` cap, so jobs survive outages for as long as `max_job_age` (7 days from first observation), the backstop against misclassification added in review; the non-network policy is unchanged (verified by the untouched pre-existing actor tests). Coverage: 7 new in-source unit tests in `upload/errors.rs`, 5 new actor tests (including tail-requeue ordering, the flat-cooldown sequence, and the age backstop), 1 new executor test, and negative-classification asserts on the three transfer failure tests.

## Context and Orientation

All paths are relative to this repository's root. Read `AGENTS.md` first: import ordering (standard / internal / external groups with comments), error conventions, and `scripts/test.sh` usage all apply.

The upload pipeline: the scanner emits stable-file events → `agent/src/workers/scan_upload_bridge.rs` enqueues a `Job` → the uploader actor (`agent/src/upload/uploader.rs`) pops it and drives `LiveExecutor::upload()` (`agent/src/upload/executor.rs`) = `create_upload` (backend HTTP) → `transfer` (S3/GCS SDK, `agent/src/upload/transfer.rs`) → `confirm_upload` (backend HTTP).

Error machinery: every error type derives `thiserror::Error` and implements the `crate::errors::Error` trait (`agent/src/errors/mod.rs`), which has default methods `code()`, `http_status()`, `params()`, `is_network_conn_err() -> false`, and `is_terminal() -> false`. Aggregating enums use the `crate::impl_error!` macro, which delegates all five methods to the active variant. Network classification is structural (not message parsing) and already exists at every layer the upload path touches:

- Backend HTTP (`agent/src/http/errors.rs`): `TimeoutErr` and `ReqwestErr { kind: Connection }` return `is_network_conn_err() == true`; `HTTPErr` delegates via `impl_error!`. `MockErr { is_network_conn_err: bool }` exists for tests.
- S3 (`agent/src/s3/errors.rs`): `S3Err::ConnectionErr` classifies true; `impl_error!` delegates.
- GCS (`agent/src/gcs/errors.rs`): `GcsErr::ConnectionErr` classifies true; `impl_error!` delegates.
- Authn (`agent/src/authn/errors.rs`): `AuthnErr` wraps `HTTPErr` and delegates via `impl_error!`, so token-fetch failures classify correctly. `AuthnErr::MockError { is_network_conn_err }` exists for tests.

The one gap is the executor boundary, `agent/src/upload/errors.rs`:

- `ExecutorErr { source: Box<dyn std::error::Error + Send + Sync>, terminal_status: Option<HTTPCode>, trace }`. Its `Error` impl overrides only `is_terminal()`; `is_network_conn_err()` falls back to the default `false`, erasing the classification present in every wrapped type.
- `executor_err(source)` wraps anything convertible to a boxed `std::error::Error` (including `&str` messages) with `terminal_status: None`.
- `classified_executor_err(source)` requires `source: crate::errors::Error` and captures `terminal_status` at wrap time. Used today by `create_upload`/`confirm_upload` in `agent/src/upload/executor.rs`.
- Two call sites wrap classifying sources with the *unclassified* helper and therefore erase network classification: `LiveExecutor::token()` (`agent/src/upload/executor.rs`, wraps `AuthnErr`) and the three typed-error sites in `agent/src/upload/transfer.rs` (`s3::S3Err` from the S3 put, `gcs::GcsErr` from GCS store build and put).

The actor policy, `Worker::run_round` in `agent/src/upload/uploader.rs`: `entry.attempts += 1` at the top of every attempt regardless of cause; up to `in_place_attempts` (3) attempts per round with exponential backoff sleeps (`cooldown::calc(&backoff, attempt_this_round - 1)`, defaults base 10s / growth 2 / cap 120s) between them; round-ending failure requeues at the tail; at `max_total_attempts` (9) the job is dropped with an error log. `terminal_status()` failures drop immediately (unchanged by this plan). Sleeps go through `self.sleep_fn` and `run_until_shutdown`, which keeps the actor responsive to commands and shutdown; tests inject a recording sleep function.

Policy precedent to mirror: `deploy/fsm.rs` line ~196 `should_bump_attempts(e) = !e.is_network_conn_err()`; `sync/syncer.rs::handle_sync_failure` — flat `TimeDelta::seconds(self.backoff.base_secs)` cooldown and `debug!` on network errors, exponential + `error!` + streak bump otherwise; `workers/mqtt.rs::handle_error` — no streak bump and `debug!` on network errors.

Test infrastructure: run everything via `./scripts/test.sh` (wraps `RUST_LOG=off cargo test --features test`; the feature flag is required). Integration tests live in `agent/tests/` mirroring `agent/src/`; `agent/tests/upload/uploader.rs` drives the actor through `agent/tests/mocks/upload_executor.rs` (`MockUploadExecutor` with scripted `MockStep::{Ok, Err, TerminalErr, Hang}` results and a started-notification channel). `agent/src/s3/errors.rs`, `agent/src/gcs/errors.rs`, and `agent/src/scan/errors.rs` show the in-source `#[cfg(test)] mod tests` pattern for error-type tests. `deploy/fsm.rs` tests show the `MockError { network_err }` pattern for attempt-bump tests.

## Plan of Work

Milestone 1 — preserve classification across the executor boundary.

In `agent/src/upload/errors.rs`: add field `pub is_network_conn_err: bool` to `ExecutorErr` and override `fn is_network_conn_err(&self) -> bool` in its `crate::errors::Error` impl to return it (next to the existing `is_terminal` override). `executor_err()` sets the field to `false` (its sources are strings or opaque errors with no classification). `classified_executor_err()` captures `source.is_network_conn_err()` before boxing, exactly as it already captures `terminal_status`. `UploadErr` needs no change — `impl_error!` already delegates `is_network_conn_err()` to the variant.

Switch the classification-erasing call sites to `classified_executor_err`: in `agent/src/upload/executor.rs`, `token()` changes `map_err(executor_err)` → `map_err(classified_executor_err)` (`AuthnErr` implements `crate::errors::Error`); in `agent/src/upload/transfer.rs`, the S3 `store.put(...).map_err(executor_err)`, the GCS `gcs_store(...).await.map_err(executor_err)`, and the GCS `store.put(...).map_err(executor_err)` all change to `classified_executor_err` (import it alongside `executor_err`). The three `executor_err("...")` string sites and the `SchemeUnknown` site stay as-is — a missing-credentials or unknown-scheme failure is not a network error.

Update the two construction sites of `ExecutorErr` in tests (`agent/tests/upload/uploader.rs::scripted_err` and `agent/tests/mocks/upload_executor.rs`) for the new field, and add unit tests (see Validation).

Milestone 2 — uploader policy in `agent/src/upload/uploader.rs`.

Rework `Worker::run_round` so attempt accounting mirrors `should_bump_attempts`:

- Move the `entry.attempts += 1` bump from the top of the loop to after the outcome is known. On `Succeeded`, bump then log (preserves "on attempt N" accuracy in `log_success`). On `Failed(err)`: bump only when `!err.is_network_conn_err()`, and do the bump *before* the terminal-status and `max_total_attempts` checks so `log_terminal_drop`/`log_dropped` report accurate counts. The pre-attempt `info!` log uses `entry.attempts + 1` as the displayed attempt number.
- Network-classified failure branch (checked after the existing `terminal_status()` check is irrelevant — terminal and network never coincide, but keep the terminal check first so ordering is unchanged for non-network errors): log at `debug!` (e.g. "upload: network connection error for file {file}; not counting attempt: {err:?}" — mirroring `workers/mqtt.rs` and `sync/syncer.rs`), requeue the entry at the tail via the existing `self.requeue(entry)`, then sleep a flat `Duration::from_secs(self.options.backoff.base_secs.max(0) as u64)` through `self.sleep_fn` + `run_until_shutdown` (returning `Flow::Shutdown` if shutdown arrives mid-sleep), then return `Flow::Continue`. The round ends immediately: no in-place retries on network failures. Add a small helper (e.g. `await_network_cooldown(&mut self) -> Flow`) beside `await_next_round` rather than inlining.
- Non-network path is byte-for-byte the same policy as today: `warn!` per attempt, `max_total_attempts` drop with `error!` via `log_dropped`, in-place exponential backoff via `await_next_round`, tail requeue at round end. Because network failures never bump `entry.attempts`, they can never trigger the `max_total_attempts` drop — jobs are never dropped due to network-classified failures.

Add `MockStep::NetworkErr` to `agent/tests/mocks/upload_executor.rs` producing `UploadErr::ExecutorErr(ExecutorErr { is_network_conn_err: true, terminal_status: None, .. })`, and add the actor tests listed in Validation.

Milestone 3 — audit confirmation and validation.

Audit of error types/wrappers added or changed since tag `v0.9.0` (`git diff v0.9.0..HEAD --stat`), performed 2026-07-20 at HEAD b068717; re-verify during implementation in case new commits land:

- `agent/src/upload/errors.rs` `ExecutorErr`/`executor_err` — ERASES classification. Fixed by Milestone 1 (the core of this plan).
- `agent/src/upload/executor.rs` `token()` — wraps classifying `AuthnErr` with the unclassified helper. Fixed by Milestone 1.
- `agent/src/upload/transfer.rs` — wraps classifying `s3::S3Err`/`gcs::GcsErr` with the unclassified helper (3 sites). Fixed by Milestone 1.
- `agent/src/s3/errors.rs`, `agent/src/gcs/errors.rs` — `ConnectionErr` classifies true; `impl_error!` delegates; both have mapping tests. No fix needed.
- `agent/src/scan/errors.rs` `ScanErr` — no variant wraps a network-classifying source (scanning is local filesystem work); `impl_error!` delegates regardless. No fix needed.
- `agent/src/http/uploads.rs` — returns `HTTPErr` unwrapped. No fix needed.
- `agent/src/http/retry.rs` `with_retry` — generic over `E: crate::errors::Error`, returns the error unchanged. No fix needed.
- `agent/src/disk/errors.rs`, `agent/src/filesys/errors.rs`, and the changed aggregating enums in `deploy/`, `server/`, `services/`, `sync/` — all use `impl_error!`; none wraps a classifying source behind a non-delegating struct. No fix needed.
- Actor-channel errors (`cache::errors::SendActorMessageErr`/`ReceiveActorMessageErr`, reused by upload and scan) — wrap tokio channel errors, not network sources; the default `false` is correct. No fix needed.
- PR #178 (upload attempt-deadline): NOT merged as of HEAD b068717 — the commit log jumps #177 → #179, and `grep -rn deadline agent/src/upload/` finds nothing. If it merges while this plan is in flight, audit its error variant: a deadline/attempt-timeout expiry must NOT classify as a network error by itself — it is a local time-budget policy decision, and classifying it network would exempt genuinely stalled jobs from failure accounting forever. If the deadline error captures the underlying attempt failure, it may delegate `is_network_conn_err()` to that source; a bare expiry counts as a real failure. Record the resolution in the Decision Log.

Then run full local validation and CI preflight (see Concrete Steps and Validation).

## Interfaces and Dependencies

No new dependencies. Changed shape (all in `agent/src/upload/errors.rs`):

    pub struct ExecutorErr {
        pub source: Box<dyn std::error::Error + Send + Sync>,
        pub terminal_status: Option<HTTPCode>,
        pub is_network_conn_err: bool,   // new
        pub trace: Box<Trace>,
    }

`executor_err` and `classified_executor_err` keep their signatures. `UploaderOptions` is unchanged — the flat cooldown reuses `backoff.base_secs`.

## Concrete Steps

All commands run from the repository root (`repos/agent` checkout). Do not commit from the workbench root.

Milestone 1:

1. Edit `agent/src/upload/errors.rs`, `agent/src/upload/executor.rs`, `agent/src/upload/transfer.rs` as described in Plan of Work; update the `ExecutorErr` construction sites in `agent/tests/upload/uploader.rs` and `agent/tests/mocks/upload_executor.rs`.
2. Add an in-source `#[cfg(test)] mod tests` to `agent/src/upload/errors.rs` (pattern: `agent/src/scan/errors.rs`) covering the cases in Validation.
3. Run `./scripts/test.sh` — expect all tests to pass, including the new ones. A quick spot check: `cargo test --features test upload::errors` should list the new tests.
4. Commit: `git add -A && git commit -m "fix(upload): preserve network-error classification across the executor boundary"` (end the message body with the Co-Authored-By trailer per repo convention).

Milestone 2:

5. Edit `agent/src/upload/uploader.rs` (`run_round`, new cooldown helper, logging) and `agent/tests/mocks/upload_executor.rs` (`MockStep::NetworkErr`); add the actor tests to `agent/tests/upload/uploader.rs`.
6. Run `./scripts/test.sh` — expect all tests green; the new actor tests fail if run against the pre-Milestone-2 worker (verify at least `network_failures_never_drop_job` does, by stashing the src change once, as a fail-before/pass-after check).
7. Commit: `git add -A && git commit -m "fix(upload): exempt network errors from attempt accounting with flat cooldown"`.

Milestone 3:

8. Re-run the audit: `git log --oneline v0.9.0..HEAD` and `git diff <last-audited-sha>..HEAD --stat -- agent/src` for anything new since b068717; apply the PR #178 guidance if a deadline variant now exists. Record findings in Surprises & Discoveries / Decision Log.
9. Run `./scripts/update-deps.sh && ./scripts/lint.sh` (lint auto-fixes; re-check `git status` afterward) and `./scripts/preflight.sh` — expect lint, tests, and coverage gates all green. `agent/src/upload/.covgate` is 96.00; the new branches must be covered by the Milestone 1–2 tests. Do NOT adjust `.covgate` values for packages this plan does not touch — `agent/src/workers/.covgate` in particular has a known pre-existing local-vs-CI coverage gap (historically ~83.13 local vs 83.21 required; the file has since changed) and must not be "fixed" here.
10. Commit any audit/lint fallout: `git add -A && git commit -m "chore(upload): post-v0.9.0 network-error propagation audit fixes"` (skip if empty).
11. Push the branch and open a draft PR against `main`. Watch the CI run on the pushed head.

## Validation and Acceptance

Unit tests — classification propagation (in `agent/src/upload/errors.rs` tests):

- `classified_executor_err(HTTPErr::MockErr { is_network_conn_err: true })` → `UploadErr::is_network_conn_err() == true`, `terminal_status() == None`.
- `classified_executor_err(HTTPErr::MockErr { is_network_conn_err: false })` → `is_network_conn_err() == false`.
- `classified_executor_err(s3::S3Err::ConnectionErr { .. })` → `true`; same for `gcs::GcsErr::ConnectionErr` and `authn::AuthnErr::MockError { is_network_conn_err: true }` (covers the token path's source type).
- `executor_err("some message")` → `is_network_conn_err() == false`.
- A terminal source (e.g. `RequestFailed` with status 400) through `classified_executor_err` still yields `terminal_status() == Some(400)` and `is_network_conn_err() == false` (regression guard for PR #179 behavior).

Actor tests (in `agent/tests/upload/uploader.rs`, using `MockStep::NetworkErr` and the recording `sleep_fn`):

- `network_failures_never_drop_job`: script 12 `NetworkErr` (exceeds `max_total_attempts` 9) then `Ok`; expect 13 recorded calls for the same job and eventual success — the job is never dropped.
- `network_failure_uses_flat_cooldown`: with backoff base 1s / growth 2, script `NetworkErr, NetworkErr, Ok`; expect recorded sleeps `[1s, 1s]` (flat), not the exponential `[1s, 2s]`.
- `network_failure_requeues_at_tail`: job A network-fails while job B is queued; expect call order `A, B, A` (B is not head-of-line blocked).
- `network_failures_do_not_consume_attempt_budget`: interleave `NetworkErr` steps among 9 non-network `Err` steps; expect the job to be dropped only after the 9th *non-network* failure, with the network attempts extra.
- Existing tests `global_attempt_cap_drops_job`, `retry_backoff_follows_expected_sequence`, and `terminal_failure_drops_job_without_requeue` still pass unchanged (non-network policy is untouched).

Executor/transfer coverage for the Milestone 1 call-site switches: in `agent/tests/upload/executor.rs`, a token-manager failure with `AuthnErr::MockError { is_network_conn_err: true }` surfaces from `upload()` with `is_network_conn_err() == true`; if the existing transfer test harness can drive a connection failure, assert the same through `SdkTransfer` — otherwise the errors.rs unit tests wrapping `S3Err::ConnectionErr`/`GcsErr::ConnectionErr` stand as the coverage for those sites.

Acceptance:

- `./scripts/test.sh` passes with the new tests present; `./scripts/preflight.sh` passes locally (lint + covgates).
- Log-level behavior: network-classified attempt failures emit `debug!` only; non-network failures keep `warn!` per attempt and `error!` on drop.
- **CI gate: preflight must report CLEAN — the CI workflows on the pushed branch head must be green — before the PR leaves draft or this task is reported complete.** A red or pending CI run is not acceptance.

## Idempotence and Recovery

All edits are plain source changes on a feature branch; every step can be re-run safely. `./scripts/test.sh`, `lint.sh`, and `preflight.sh` are read-only apart from lint auto-fixes (re-check `git status` after lint). Each milestone is one commit, so a bad milestone is recoverable with `git revert <sha>` (or `git reset --hard` to the previous milestone before pushing). No migrations, no destructive steps. If the branch conflicts with `main` (e.g. PR #178 merges), rebase, re-run the Milestone 3 audit against the new HEAD, and re-run the full validation.
