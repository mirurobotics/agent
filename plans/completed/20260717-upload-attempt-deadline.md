# Add a size-scaled deadline around upload attempts

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (this repo) | read-write | Timeout around upload attempts in `agent/src/upload/`, error variant, tests, doc-comment updates |

This plan lives in this repo's `plans/` because all code changes are here. Work happens on branch `feat/upload-attempt-deadline` (already created from `main` at `c1f68d0`).

## Purpose / Big Picture

Today, a silently dead connection during a GCS object transfer stalls the upload actor forever: `gcs::Store::put` (`agent/src/gcs/mod.rs`) intentionally carries no timeout — its doc comment says "callers that need a bound must enforce their own size-scaled deadline" — and no caller does. The actor's round never ends, backoff/retry never fires, and the queue backs up toward `QueueFullErr` (capacity 1024). S3 puts get implicit stall protection from AWS SDK defaults, so the gap is primarily the GCS path.

After this change every upload attempt is wrapped in `tokio::time::timeout` with a deadline scaled to the file size: a floor constant plus a per-byte allowance (a minimum-throughput assumption). Deadline expiry surfaces as a new retryable `UploadErr` variant, so the existing in-place-attempt/backoff/requeue machinery in the actor handles it like any other failed attempt. Observable outcome: an upload that hangs forever now fails after its deadline, is retried with backoff, and eventually either succeeds or is dropped at the global attempt cap — the actor and its queue keep flowing.

Deadline formula, with defaults:

    deadline = attempt_timeout_floor + ceil(job.size / attempt_timeout_min_bytes_per_sec)
    attempt_timeout_floor            = 120 s
    attempt_timeout_min_bytes_per_sec = 64 KiB/s (65,536)

Examples: 0 B → 120 s; 1 MiB → 136 s; 100 MiB → ~27 min; 1 GiB → ~4.7 h. This is a liveness bound (detect dead connections), not a performance SLA — a false timeout discards transfer progress, so the throughput assumption errs generous while still guaranteeing every attempt terminates.

## Progress

- [x] Milestone 1: deadline formula + retryable timeout error variant, with unit tests. (`feat(upload): add attempt deadline formula and timeout error variant`; formula unit test `attempt_deadline_formula` landed with the test commit below.)
- [x] Milestone 2: wire `tokio::time::timeout` into the actor's attempt, with behavioral tests. (`feat(upload): bound upload attempts with a size-scaled deadline`; acceptance test `hung_attempt_times_out_and_is_retried` landed with the test commit below.)
- [x] Milestone 3: doc-comment updates, covgate, preflight CLEAN. (`docs(upload): document the size-scaled attempt deadline contract`; `test(upload): cover attempt deadline formula and hung-attempt retry`; local `./scripts/preflight.sh` clean — CI on the pushed branch head is verified on the draft PR.)

## Surprises & Discoveries

- The first full `./scripts/preflight.sh` run failed in its tests/covgate component while a direct `./scripts/covgate.sh` rerun and a second full preflight both passed on identical code — a transient flake from the four parallel jobs (clippy `--fix` build plus two coverage builds) contending, not a real failure. Rerun the failing script alone before treating a parallel-preflight failure as a regression.

## Decision Log

(Add entries as you go. Pre-authoring decisions that shaped this plan are recorded below so a novice understands the "why".)

- Decision: wrap the deadline at the actor seam (`Worker::attempt_upload` in `agent/src/upload/uploader.rs`), around the whole `executor.upload(&job)` future, rather than inside `LiveExecutor::upload` or around `ObjectTransfer::transfer`.
  Rationale: (1) one uniform bound covers the whole attempt (create → transfer → confirm) for every `UploadExecutor` implementation; the control-plane HTTP calls are already individually bounded (10 s per request, ≤3 attempts — see Context), so the floor absorbs them. (2) `job.size` is in scope at the actor (`entry.job.size`). (3) The deadline is attempt policy, and all other attempt policy (`in_place_attempts`, `max_total_attempts`, `backoff`) already lives in `UploaderOptions`, which tests already pin. (4) The test seam already exists: `MockUploadExecutor`'s `MockStep::Hang` plus `#[tokio::test(start_paused = true)]`; the executor-level seam would require adding hang support to `MockObjectTransfer`. (5) The `UploadExecutor` trait already documents that the actor drops in-flight futures on shutdown ("Cancel safety" section), so a timeout dropping the future is the same, already-honored contract.
  Date/Author: 2026-07-17, planning session.
- Decision: expose the deadline computation as `UploaderOptions::attempt_deadline(&self, size: u64) -> Duration` (public method) instead of a private free function.
  Rationale: tests live in `agent/tests/` (external-crate integration tests) and cannot reach private items; a method on the options struct keeps the formula next to its knobs and directly unit-testable.
  Date/Author: 2026-07-17, planning session.
- Decision: defaults floor = 120 s, min throughput = 64 KiB/s.
  Rationale: floor must cover worst-case bounded control-plane work for a tiny file — token fetch plus create and confirm at up to 3 HTTP attempts × 10 s each plus ~1 s retry delays ≈ 70 s — with headroom for TLS/connection setup; 64 KiB/s is well below any plausible sustained device uplink (fleet devices may be on LTE), so slow-but-alive transfers don't get killed, while dead connections always terminate.
  Date/Author: 2026-07-17, planning session.
- Decision: tests landed as one follow-up commit (`test(upload): cover attempt deadline formula and hung-attempt retry`) instead of being folded into the milestone-1/2 commits.
  Rationale: the implementation pipeline runs source, a fresh-context refine pass, then tests; the branch content is identical to the plan's, only the commit slicing differs.
  Date/Author: 2026-07-17, implementation session.

## Outcomes & Retrospective

Implemented exactly as planned, with zero source deviations (the refine pass over the diff reported no findings):

- Every upload attempt is now bounded by `tokio::time::timeout` at the actor seam with `UploaderOptions::attempt_deadline` (floor 120 s + 1 s per 64 KiB); expiry surfaces as the retryable `UploadErr::AttemptTimeoutErr` and flows through the unchanged backoff/requeue/attempt-cap machinery.
- Acceptance proof: `hung_attempt_times_out_and_is_retried` (paused tokio clock) shows a never-completing attempt ending at its 2 s virtual deadline and the same job retried through the normal backoff path; the pre-existing shutdown-race tests pass unchanged.
- Validation: full suite 1515 passed / 0 failed; covgate upload module 96.81% against the 96.00 gate (no gate lowered); `./scripts/preflight.sh` clean locally. CI (`lint`, `test`, `tools`) is verified on the pushed head of `feat/upload-attempt-deadline` via the draft PR.
- Retrospective: the plan's file-level prescriptions (exact fields, formula, error struct, test scripts) made implementation mechanical; the only friction was a transient parallel-preflight flake (see Surprises).

## Context and Orientation

The agent is a Rust daemon. The upload feature lives in `agent/src/upload/` with tests in `agent/tests/upload/` (integration tests importing the crate as `miru_agent`) and shared test doubles in `agent/tests/mocks/`. Relevant pieces, all paths repo-relative:

- `agent/src/upload/uploader.rs` — the actor. `UploaderOptions` (top of file) holds the tunables with doc-commented fields and a `Default` impl (`queue_capacity: 1024`, `in_place_attempts: 3`, `max_total_attempts: 9`, `backoff`). `Worker::run_round` drives up to `in_place_attempts` attempts with backoff sleeps between, requeues at the tail on a round-ending failure, and drops the job at `max_total_attempts`. `Worker::attempt_upload` clones the executor and job, then drives `executor.upload(&job)` inside `run_until_shutdown`, which races the future against actor commands via `tokio::select!` and drops it if a shutdown arrives. Backoff sleeps go through the injected `sleep_fn: Fn(Duration) -> Fut` seam (production passes `tokio::time::sleep`; tests pass a no-op or a recording closure).
- `agent/src/upload/executor.rs` — `UploadExecutor` trait (`fn upload(&self, job: &Job) -> impl Future<Output = Result<(), UploadErr>> + Send`) and the production `LiveExecutor`, which creates the upload via the backend HTTP API, transfers via `ObjectTransfer`, confirms, then applies the delete policy. The trait's doc comment has a "Cancel safety" section stating implementations must tolerate being dropped at any await point.
- `agent/src/upload/transfer.rs` — `ObjectTransfer` trait and production `SdkTransfer` calling `s3::Store::put` / `gcs::Store::put`.
- `agent/src/upload/errors.rs` — error conventions: each concrete error is a `pub struct` deriving `thiserror::Error` with a `trace: Box<Trace>` field (built via `crate::trace!()`) and an `impl crate::errors::Error`; the `UploadErr` enum aggregates them with `#[error(transparent)]` variants and the `crate::impl_error!` macro; `executor_err` is the helper-constructor pattern to copy. The actor treats any error returned from `executor.upload` as retryable — `run_round` does not discriminate variants — so a new variant is automatically retried.
- `agent/src/upload/job.rs` — `Job` with `pub size: u64` (from file metadata) and `pub file: File` (implements `Display`).
- `agent/src/gcs/mod.rs` — `Store::put` doc comment (final paragraph, ~line 197) is the contract this plan takes up: "The upload itself carries no timeout … callers that need a bound must enforce their own size-scaled deadline (e.g. `tokio::time::timeout`) around it." Nearby constants (`CONTROL_ATTEMPT_TIMEOUT`, `READ_IDLE_TIMEOUT`) show the doc style for timeout constants: what is bounded and why.
- `agent/src/http/` — control-plane requests default to a 10 s per-request timeout (`DEFAULT_TIMEOUT` in `request.rs`) and `with_retry` (`retry.rs`) makes at most 3 attempts on network errors. This bounds `create_upload`/`confirm_upload` and informs the floor.
- Tests: `agent/tests/upload/uploader.rs` shows the patterns to mirror — `spawn_uploader` with a no-op `sleep_fn`, `MockUploadExecutor::new()` returning a `started_rx` notified at the start of each `upload` call, `MockStep::{Ok, Err, Hang(oneshot::Receiver)}` scripting, a `timed()` 5 s wrapper, and `retry_backoff_follows_expected_sequence` which pins `UploaderOptions` and records `sleep_fn` durations. `agent/tests/sync/syncer.rs` shows `#[tokio::test(start_paused = true)]` usage (paused tokio clock; the runtime auto-advances to the next timer when all tasks are idle).
- Conventions: `AGENTS.md` — import ordering (std/internal/external groups with comments), error conventions above, tests via `./scripts/test.sh` (sets `--features test`; required), coverage via `./scripts/covgate.sh` (`agent/src/upload/.covgate` requires 96.00), lint via `./scripts/lint.sh`, all combined in `./scripts/preflight.sh`. The field-by-field assert linter flags 4+ `assert_eq!` on fields of one variable in a test.

## Plan of Work

Milestone 1 — formula and error variant.

In `agent/src/upload/uploader.rs`, add two doc-commented fields to `UploaderOptions` and their defaults in `impl Default`:

    /// Fixed floor of the per-attempt upload deadline. Covers control-plane
    /// RPCs (create/confirm, each bounded at 3 × 10s attempts) and connection
    /// setup so tiny files are never starved.
    pub attempt_timeout_floor: Duration,          // default: Duration::from_secs(120)
    /// Minimum-throughput assumption scaling the per-attempt deadline with
    /// file size. Deliberately far below any plausible sustained uplink: a
    /// false timeout discards transfer progress, so this errs generous while
    /// still guaranteeing every attempt terminates.
    pub attempt_timeout_min_bytes_per_sec: u64,   // default: 64 * 1024

Add a public method with the formula (saturating; guard division by zero with `.max(1)`; round up with `div_ceil`):

    impl UploaderOptions {
        /// Deadline for one upload attempt of `size` bytes: floor plus a
        /// per-byte allowance at the minimum-throughput assumption.
        pub fn attempt_deadline(&self, size: u64) -> Duration {
            let bps = self.attempt_timeout_min_bytes_per_sec.max(1);
            self.attempt_timeout_floor
                .saturating_add(Duration::from_secs(size.div_ceil(bps)))
        }
    }

In `agent/src/upload/errors.rs`, following the existing struct pattern (`thiserror::Error`, `trace` field, `impl crate::errors::Error`), add:

    #[derive(Debug, thiserror::Error)]
    #[error("upload attempt for file {file} ({size} bytes) exceeded its {deadline:?} deadline")]
    pub struct AttemptTimeoutErr {
        pub file: String,
        pub size: u64,
        pub deadline: std::time::Duration,
        pub trace: Box<Trace>,
    }

Add an `AttemptTimeoutErr(AttemptTimeoutErr)` transparent variant to `UploadErr`, list it in the `crate::impl_error!` block, and add a `pub(crate) fn attempt_timeout_err(file: String, size: u64, deadline: std::time::Duration) -> UploadErr` helper mirroring `executor_err` (takes plain data so `errors.rs` keeps not depending on `job.rs`; use the full `std::time::Duration` path or add the std import group per the file's import-ordering convention).

Tests (milestone 1), in `agent/tests/upload/uploader.rs`: a plain `#[test]` for `attempt_deadline` — size 0 returns exactly the floor; size = 1 adds a full second (`div_ceil` rounds up); size = exact multiple of `bps` adds `size/bps` seconds; `attempt_timeout_min_bytes_per_sec: 0` does not panic. Pin a small custom `UploaderOptions` so assertions are independent of production defaults (mirror `retry_backoff_follows_expected_sequence`).

Milestone 2 — wire the timeout into the actor.

In `Worker::attempt_upload` (`agent/src/upload/uploader.rs`), compute the deadline before building the future and wrap the executor call:

    let executor = self.executor.clone();
    let job = entry.job.clone();
    let deadline = self.options.attempt_deadline(job.size);
    let attempt = async move {
        match tokio::time::timeout(deadline, executor.upload(&job)).await {
            Ok(result) => result,
            Err(_) => Err(attempt_timeout_err(job.file.to_string(), job.size, deadline)),
        }
    };
    match self.run_until_shutdown(attempt).await { ... unchanged ... }

No `run_round` changes: a timeout is just another `AttemptOutcome::Failed`, so backoff, requeue-at-tail, and the `max_total_attempts` drop all apply unchanged. `tokio::time::timeout` keeps the future `Send`, so no bound changes on `Worker`.

Tests (milestone 2), in `agent/tests/upload/uploader.rs`:

- `hung_attempt_times_out_and_is_retried` — `#[tokio::test(start_paused = true)]`. Script `MockStep::Hang(release_rx)` where `release_tx` is held but never fired, then `MockStep::Ok`. Spawn with pinned options — `attempt_timeout_floor: Duration::from_secs(1)`, `attempt_timeout_min_bytes_per_sec: 64 * 1024` (so `make_job`'s `size: 42` yields a 2 s deadline, safely under the `timed()` helper's 5 s cap) — and a `sleep_fn` that records durations and returns immediately (backoff then consumes no virtual time, so the attempt deadline is the nearest pending timer and the paused clock auto-advances to it once tasks idle). Enqueue; await `started_rx` twice (initial attempt, then the retry after the timeout fires). Assert: two recorded calls for the same job; the recorded backoff sleeps are non-empty (the timeout took the normal in-place-retry path); then shutdown and join. This test fails before the milestone-2 change (the first attempt hangs forever; the second `started_rx.recv()` trips the `timed()` 5 s guard) and passes after.
- `attempt_completes_within_deadline_is_unaffected` — with default options, a scripted `MockStep::Ok` job succeeds with exactly one recorded call (guards against the wrapper changing happy-path behavior). The existing `processes_enqueued_job` already covers this; extend it only if reviewers want an explicit paused-clock variant, otherwise skip as redundant.
- Existing shutdown tests (`shutdown_during_in_flight_upload_returns_promptly`, `worker_exits_when_handles_dropped_during_in_flight_upload`) must keep passing unchanged — they prove the timeout wrapper did not break the shutdown race.

Milestone 3 — documentation and validation.

- `agent/src/gcs/mod.rs`, `Store::put` doc comment final paragraph: keep the contract sentence but note it is now honored — e.g. "… callers that need a bound must enforce their own size-scaled deadline (e.g. `tokio::time::timeout`) around it. The upload actor does exactly that: `Worker::attempt_upload` in `agent/src/upload/uploader.rs` bounds every attempt with a floor-plus-per-byte deadline (`UploaderOptions::attempt_deadline`)."
- `agent/src/upload/executor.rs`, `UploadExecutor` trait "Cancel safety" doc: extend "The actor drops an in-progress `upload` future on shutdown" to "on shutdown or when the attempt deadline expires".
- `agent/src/upload/uploader.rs`, `attempt_upload` doc comment: mention the size-scaled deadline and that expiry surfaces as `AttemptTimeoutErr`, retried like any failure.
- Run the full validation below; fix anything it surfaces.

## Concrete Steps

All commands from the repo root `/home/ben/miru/workbench2/repos/agent` on branch `feat/upload-attempt-deadline`.

1. Milestone 1 edits (`agent/src/upload/uploader.rs`, `agent/src/upload/errors.rs`) and formula tests in `agent/tests/upload/uploader.rs`. Note: any test constructing `UploaderOptions { .. }` field-by-field must add the two new fields or use `..UploaderOptions::default()` (the existing tests already use struct-update syntax, so expect no fallout).

       ./scripts/test.sh

   Expect: all tests pass, including the new `attempt_deadline` cases. Commit:

       git add -A && git commit -m "feat(upload): add attempt deadline formula and timeout error variant"

2. Milestone 2 edits (`Worker::attempt_upload`) and behavioral tests.

       ./scripts/test.sh

   Expect: all tests pass; `hung_attempt_times_out_and_is_retried` passes (and fails if the timeout wrapping is reverted). Commit:

       git add -A && git commit -m "feat(upload): bound upload attempts with a size-scaled deadline"

3. Milestone 3 doc updates, then full local validation:

       ./scripts/preflight.sh

   Expect: lint, tests, covgate (upload module ≥ 96.00), and tools checks all green. Commit:

       git add -A && git commit -m "docs(upload): document the size-scaled attempt deadline contract"

4. Push and confirm CI on the branch head:

       git push -u origin feat/upload-attempt-deadline
       gh run list --branch feat/upload-attempt-deadline --limit 1
       gh run watch <run-id-from-previous-command> --exit-status

   Expect: the CI workflow (`.github/workflows/ci.yml`: `lint`, `test`, `tools`) green on the pushed head.

## Validation and Acceptance

- From `/home/ben/miru/workbench2/repos/agent`: `./scripts/test.sh` passes with zero failures. The new test `hung_attempt_times_out_and_is_retried` fails before the milestone-2 change (hangs into the 5 s test guard) and passes after — this is the acceptance proof that a silently dead connection no longer stalls the actor and that expiry is retried through the normal backoff/requeue path.
- `UploaderOptions::default().attempt_deadline(0)` is exactly 120 s and `attempt_deadline(64 * 1024)` is 121 s (formula acceptance).
- Existing shutdown-responsiveness tests pass unchanged.
- `./scripts/covgate.sh` passes; `agent/src/upload/.covgate` (96.00) is not lowered.
- Preflight must report CLEAN before delivery: `./scripts/preflight.sh` fully green locally, and CI (`lint`, `test`, `tools` jobs) green on the pushed branch head. The PR must not leave draft — and the task must not be reported complete — until CI is green on the branch head.

## Idempotence and Recovery

All steps are additive code edits plus test runs — safe to re-run; `./scripts/test.sh`, `./scripts/covgate.sh`, and `./scripts/preflight.sh` are read-only with respect to the working tree. Each milestone is one commit, so a bad step is recovered with `git revert <sha>` (or `git reset --hard` to the previous milestone before pushing). No migrations, no destructive operations. If the paused-clock test proves flaky under CI parallelism (it should not — it uses virtual time only), fall back to releasing the hang explicitly via a second timer rather than relaxing the deadline constants.
