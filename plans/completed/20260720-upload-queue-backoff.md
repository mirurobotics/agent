# Upload queue backoff: single attempt per pop, deferred per-job retry, no head-of-line blocking

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (repo root of mirurobotics/agent) | read-write | Rework the upload queue retry model in `agent/src/upload/queue.rs` and `agent/src/upload/uploader.rs`, update the spawn wiring in `agent/src/app/state.rs`, and update mirror tests under `agent/tests/upload/` and `agent/tests/workers/scan_upload_bridge.rs`. |

This plan lives in the agent repo because every code change is inside it. Work happens on branch `fix/upload-queue-backoff` (already checked out, based on `main`). All commands run from the repo root.

## Purpose / Big Picture

The uploader actor drains a FIFO queue of upload jobs. Today a failing job is retried 3 times in a row ("in-place attempts") with backoff sleeps between attempts (10 s then 20 s — the backoff resets each round, so its 120 s cap is unreachable), and those sleeps block every other job in the queue. A job is dropped after 9 total attempts, so a persistently failing upload burns all its attempts within roughly 90 seconds of backoff even though the outage (network, backend) may last hours.

After this change:

- Every pop gets exactly one executor attempt. The `in_place_attempts` knob is gone; the only retry knob left is `attempts` (total attempts per job before it is dropped), raised to 30.
- A failed job is stamped with a wall-clock "not before" time (`next_attempt_at`) computed from exponential backoff (base 10 s, growth 2, capped at 1 hour) and requeued at the tail. The worker immediately moves on to the next job — it never sleeps while holding a job.
- A job still inside its backoff window is skipped: the queue hands out the first *eligible* entry, so one long-waiting upload never blocks the others. When every queued job is waiting, the worker sleeps until the earliest becomes eligible, waking early for any command (an enqueue during that sleep is processed immediately).

With the new defaults a job that fails every attempt is retried over roughly 21 hours (10 s, 20 s, 40 s, ... 2560 s, then 1 hour flat) before being dropped, instead of 15 minutes.

Observable outcome: run `./scripts/test.sh` and see the new uploader tests prove (a) a failed job's retry runs *after* a later-enqueued job, and (b) a waiting job is skipped while a fresh enqueue is processed immediately.

## Progress

- [x] Milestone 1: queue eligibility — `next_attempt_at` on `QueueEntry`, `pop_ready`, `earliest_next_attempt`; queue tests; commit. (7dace8b source + mechanical test adaptation; new eligibility tests landed with d384644.)
- [x] Milestone 2: uploader single-attempt with deferred retry — options rename/removal, worker rewrite, `now_fn` wiring, uploader + bridge tests; commit. (3d3cd3d source, d384644 test rework; full suite 1530 passed, covgate upload 96.21 ≥ 96.00.)
- [ ] Milestone 3: preflight to CLEAN (CI green on the pushed branch head).

## Surprises & Discoveries

- Discovery: with the new model, a test using a no-op `sleep_fn` and the real clock busy-loops whenever every queued entry is inside its backoff window — `idle_wait`'s instantly-ready sleep starves tokio's current-thread test runtime so even `tokio::time::timeout` never fires (the old `global_attempt_cap_drops_job` hung this way after the worker rewrite, before its test-clock rework). The test harness now documents that `spawn_uploader` (no-op sleep) is only safe when no stamped entry is ever left waiting; other tests use the advancing test clock or a frozen clock + pending sleep.
- Discovery: the planned test inventory left the upload module at 95.45% coverage, below its 96.00 gate. One extra test (`command_pending_at_shutdown_returns_receive_err`) covering the shutdown-ack error path and the dropped-response `ReceiveActorMessageErr` branch brought it to 96.21 — the gate was not lowered.
- Discovery: `requeue_into_full_queue_drops_job` shrinks from `[A, A, A, B]` to `[A, B]` recorded calls under the new model — A's single failed attempt cannot requeue into the full queue and is dropped immediately, which is exactly the intended no-head-of-line-blocking behavior.

## Decision Log

(Add entries as work proceeds. Pre-authoring decisions that shaped this plan:)

- Decision: delete the in-place retry loop rather than defaulting `in_place_attempts` to 1.
  Rationale: the task requires removing the knob entirely, leaving `attempts` as the single retry config. Date/Author: 2026-07-20 / ben@miruml.com.
- Decision: `attempts: 30`, backoff `{ base_secs: 10, growth_factor: 2, max_secs: 3600 }`.
  Rationale: exponent is `attempts_so_far - 1`, so waits run 10, 20, ..., 2560, then cap at 3600 from the 10th failure on — ~21.4 h of retrying before drop, covering multi-hour outages. Date/Author: 2026-07-20 / ben@miruml.com.
- Decision: store the backoff deadline as a wall-clock `next_attempt_at: Option<DateTime<Utc>>` on `QueueEntry`, serialized with `#[serde(default)]`; `None` means eligible now.
  Rationale: `QueueEntry` is persisted in the queue snapshot, so an absolute timestamp survives agent restarts (a device rebooting mid-backoff resumes the wait), and `serde(default)` lets snapshots written by older agents (entries without the field) load as immediately eligible. Date/Author: 2026-07-20 / ben@miruml.com.
- Decision: the backoff exponent is derived from the job's lifetime attempt count (`entry.attempts - 1`), not reset per round.
  Rationale: rounds no longer exist; a monotonically growing wait per job is the intended "back off up to 1 hour" behavior. This intentionally changes today's reset-per-round schedule. Date/Author: 2026-07-20 / ben@miruml.com.
- Decision: inject the clock as a `now_fn` parameter on `Uploader::spawn`, mirroring the existing `sleep_fn` seam.
  Rationale: eligibility is time-based; tests need a controllable clock (a shared `DateTime<Utc>` that the test's `sleep_fn` advances) to be deterministic without real sleeping. Production passes `Utc::now`. Date/Author: 2026-07-20 / ben@miruml.com.
- Decision: the idle wait (all queued jobs in backoff) re-evaluates the queue after *every* handled command, unlike `run_until_shutdown` which keeps driving its future.
  Rationale: an enqueue during the wait must run immediately; the wait deadline is recomputed from `earliest_next_attempt` each loop iteration, so dropping and rebuilding the sleep future loses nothing. Date/Author: 2026-07-20 / ben@miruml.com.
- Decision: `idle_wait` is a single `tokio::select!` with `biased;` polling `receiver.recv()` before the sleep.
  Rationale: commands are handled first when both are ready; the biased recv cannot starve eligible entries because every handled command returns to the loop top, which pops before recv'ing again. Date/Author: 2026-07-20 / implementation.
- Decision: the idle wait converts the chrono delta via `(earliest - now).to_std().unwrap_or(Duration::ZERO)`.
  Rationale: `to_std()` errors on negative deltas, so the fallback doubles as the >= 0 clamp; deadline stamping uses `chrono::TimeDelta::seconds` to avoid clashing with the imported `std::time::Duration`. Date/Author: 2026-07-20 / implementation.

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

Repo root: the checkout of `mirurobotics/agent`, a Rust workspace whose binary crate `miru-agent` lives under `agent/`. Read `AGENTS.md` first — import ordering (standard/internal/external groups with comments), `./scripts/test.sh` usage (`--features test` is mandatory), and per-module `.covgate` coverage gates all apply. Test files under `agent/tests/` mirror `agent/src/` modules.

Key pieces, all repo-relative:

- `agent/src/upload/uploader.rs` — the uploader actor. `UploaderOptions` (lines ~33-60) currently holds `queue_capacity: usize` (default 1024), `in_place_attempts: u32` (default 3), `max_total_attempts: u32` (default 9), and `backoff: cooldown::Backoff` (default base 10 s, growth 2, max 120 s). `Worker::run` pops an entry and calls `run_round`, which loops up to `in_place_attempts` executor attempts with `await_next_round` backoff sleeps between them; on the round-ending failure it requeues at the tail with no sleep; it drops the job on a terminal HTTP status (`err.terminal_status()`) or when `entry.attempts >= max_total_attempts`. `run_until_shutdown` drives a future while serving actor commands (`Enqueue`, `Len`, `Shutdown`); `Uploader::spawn(buffer_size, executor, options, snapshot_file, sleep_fn)` builds the worker. `sleep_fn: Fn(Duration) -> Fut` is the test seam for sleeping.
- `agent/src/upload/queue.rs` — FIFO `Queue` over `VecDeque<QueueEntry>` where `QueueEntry { job: Job, attempts: u32 }` (serde-derived). `enqueue` pushes a fresh entry (attempts 0), `requeue` pushes back a popped entry preserving attempts, `pop_front` removes the head. Every mutation persists a `QueueSnapshot { entries }` through `QueueSnapshotFile` (a `SingleThreadStateFile`, plain JSON on disk via `files::write_json`; `new_with_default` falls back to writing the default when the file is unreadable).
- `agent/src/cooldown/mod.rs` — `Backoff { base_secs, growth_factor, max_secs }` (all `i64`) and `calc(&Backoff, exp: u32) -> i64` = `min(base * growth^exp, max)`. Unchanged by this plan.
- `agent/src/app/state.rs::init_uploader` (~line 212) — the only production spawn: `upload::Uploader::spawn(64, executor, upload::UploaderOptions::default(), snapshot_file, |wait| tokio::time::sleep(wait))`.
- `agent/tests/upload/uploader.rs` — actor tests driven by `agent/tests/mocks/upload_executor.rs` (`MockUploadExecutor` with scripted `MockStep::{Ok, Err, TerminalErr, Hang}` and a `started_rx` channel signaling each attempt start). `agent/tests/upload/queue.rs` — queue unit tests (its `digests` helper drains via `pop_front`). `agent/tests/workers/scan_upload_bridge.rs` — spawns a real `Uploader` in its harness (~line 70).
- The field names `in_place_attempts` / `max_total_attempts` appear only in `agent/src/upload/uploader.rs`; all other call sites use `UploaderOptions::default()` or struct-update syntax, and no docs outside `plans/completed/` mention them.

"Terminal status" means the executor classified the failure as a permanent 4xx backend rejection; those jobs are dropped without requeue today and that behavior is preserved.

Which requested behaviors already exist: "move on after a failure" exists only per-round today (after 3 in-place attempts the job is requeued at the tail); it becomes per-attempt by deleting the loop. "Skip waiting jobs" does not exist at all — backoff sleeps happen in-place while holding the job, and a requeued entry carries no deadline, so with a single attempt per pop the current code would hot-loop on a lone failing job. Both need the code changes below.

## Plan of Work

Milestone 1 — queue eligibility (`agent/src/upload/queue.rs`).

Add `pub next_attempt_at: Option<chrono::DateTime<chrono::Utc>>` to `QueueEntry` with `#[serde(default)]` (import chrono in the external-crates group). `enqueue` constructs entries with `next_attempt_at: None`. Replace `pop_front` with `pop_ready(&mut self, now: DateTime<Utc>) -> Option<QueueEntry>`: find the first index whose entry has `next_attempt_at` `None` or `<= now`, remove it with `VecDeque::remove` (preserves the order of the rest), persist, and return it; return `None` (no persist, no log) when nothing is eligible. Add `earliest_next_attempt(&self) -> Option<DateTime<Utc>>` returning the minimum effective deadline over all entries (`None` deadlines count as `DateTime::<Utc>::MIN_UTC`), or `None` when the queue is empty — the worker uses it to size the idle sleep. `requeue` is unchanged (the caller stamps the deadline). Update `agent/tests/upload/queue.rs`: switch the `digests` helper and `pop_front` tests to `pop_ready(Utc::now())`, extend the requeue tests to stamped entries, and add the new tests listed under Validation.

Milestone 2 — uploader single attempt with deferred retry (`agent/src/upload/uploader.rs`, `agent/src/app/state.rs`, tests).

In `UploaderOptions`: delete `in_place_attempts`; rename `max_total_attempts` to `attempts` (doc: total executor attempts per job before it is dropped); set defaults `attempts: 30` and `backoff: Backoff { base_secs: 10, growth_factor: 2, max_secs: 3600 }` (`queue_capacity` stays 1024).

Add a clock seam: `Uploader::spawn` and `Worker` gain a `now_fn: N` where `N: Fn() -> chrono::DateTime<chrono::Utc> + Send + Sync + 'static`, passed after `sleep_fn`. Production (`agent/src/app/state.rs::init_uploader`) passes `chrono::Utc::now`.

Rewrite the worker loop. `run` becomes: pop via `self.queue.pop_ready((self.now_fn)())` —

- `Some(entry)`: call `run_attempt(entry)` (replaces `run_round`): increment `entry.attempts`, drive one `attempt_upload` via the existing `run_until_shutdown`. On success, log and continue. On failure: drop on `terminal_status()` (unchanged); drop with `log_dropped` when `entry.attempts >= self.options.attempts`; otherwise compute `wait = cooldown::calc(&self.options.backoff, entry.attempts - 1).max(0)`, set `entry.next_attempt_at = Some(now + chrono::Duration::seconds(wait))`, log the wait, and requeue at the tail via the existing `requeue` helper. No sleeping inside `run_attempt`.
- `None` and `self.queue.is_empty()`: block on `receiver.recv()` exactly as today.
- `None` but non-empty (all entries waiting): compute `wait = (earliest_next_attempt - now)` clamped to `>= 0`, build `(self.sleep_fn)(wait)`, and `tokio::select!` it against `receiver.recv()`. Sleep completing, or any non-shutdown command (handled first), returns to the top of the loop to re-evaluate; `Shutdown` or a closed channel exits. This is a new helper (e.g. `idle_wait(&mut self, wait: Duration) -> Flow`) — deliberately *not* `run_until_shutdown`, which would keep sleeping after an enqueue. Delete `await_next_round`.

Update the two test spawn sites for the new signature: `agent/tests/upload/uploader.rs` and `agent/tests/workers/scan_upload_bridge.rs` (the bridge harness passes `chrono::Utc::now` and keeps its no-op sleep; its jobs never fail so nothing else changes). In the uploader tests add a deterministic clock harness: a `Arc<Mutex<DateTime<Utc>>>` test clock; `now_fn` reads it; the test `sleep_fn` records the requested `Duration` and advances the clock by it, so backoff waits resolve instantly and exactly. Rework/add uploader tests per Validation.

Milestone 3 — preflight. Run the repo's full preflight (every CI job: lint, tests+covgate, tools self-lint, tools covgate) and iterate until CI is green on the pushed branch head.

## Concrete Steps

All commands run from the repo root on branch `fix/upload-queue-backoff`.

Milestone 1:

1. Edit `agent/src/upload/queue.rs` and `agent/tests/upload/queue.rs` as described in Plan of Work.
2. Run the queue tests:

       RUST_LOG=off cargo test --package miru-agent --features test upload::queue

   Expect all queue tests to pass (existing count plus the new eligibility/compat tests). Note: `agent/src/upload/uploader.rs` will not compile against `pop_ready` yet if milestone 2 has not started — to keep milestone 1 self-contained, mechanically update the worker's single `pop_front` call site to `pop_ready(chrono::Utc::now())` in this milestone (behavior-identical while every entry's deadline is `None`), leaving the real worker rewrite to milestone 2.
3. Commit:

       git add -A && git commit -m "feat(upload): queue entries carry a next_attempt_at eligibility deadline"

Milestone 2:

4. Edit `agent/src/upload/uploader.rs`, `agent/src/app/state.rs`, `agent/tests/upload/uploader.rs`, `agent/tests/workers/scan_upload_bridge.rs` as described in Plan of Work.
5. Run the affected suites, then the full suite:

       RUST_LOG=off cargo test --package miru-agent --features test upload::
       RUST_LOG=off cargo test --package miru-agent --features test workers::scan_upload_bridge
       ./scripts/test.sh

   Expect zero failures.
6. Check coverage against the module gate (`agent/src/upload/.covgate` is 96.00):

       ./scripts/covgate.sh

   If the upload module dips below its gate, add the missing branch coverage (the drop paths and `idle_wait` arms are the likely gaps) rather than lowering the gate.
7. Commit:

       git add -A && git commit -m "feat(upload): single attempt per pop with deferred per-job backoff"

Milestone 3:

8. Lint locally, fixing anything reported:

       ./scripts/update-deps.sh && ./scripts/lint.sh

9. Run the full preflight (mirrors every CI job) and iterate until it is clean, then push and confirm CI is green on the branch head:

       ./scripts/preflight.sh
       git push -u origin fix/upload-queue-backoff

   Expected: preflight prints all four sections (Lint, Tests, tools lint, tools tests) with success exits, and the GitHub Actions run for the pushed head passes every job.

## Validation and Acceptance

Test inventory (all under `agent/tests/`; names indicative):

Queue (`upload/queue.rs`):

- `pop_ready` FIFO behavior with no deadlines — adapt the existing `pop_front` tests; entries pop in enqueue order.
- `pop_ready_skips_waiting_entries`: entries A (deadline in the future) then B (none); `pop_ready(now)` returns B and leaves A queued at its position; a later `pop_ready(after_deadline)` returns A.
- `pop_ready_returns_none_when_all_waiting`: single future-deadline entry; `pop_ready(now)` is `None`, `len()` unchanged, and `earliest_next_attempt()` returns that deadline (also cover the empty-queue `None` and the mixed case where a `None`-deadline entry makes the minimum `MIN_UTC`).
- `next_attempt_at_survives_reload`: requeue a stamped entry, reopen the snapshot file, and the reloaded entry carries the same deadline.
- `legacy_snapshot_without_next_attempt_at_loads`: serialize a current `QueueSnapshot` to `serde_json::Value`, delete the `next_attempt_at` key from each entry, write the JSON to the snapshot path, open via `QueueSnapshotFile::new_with_default`, and assert `pop_ready(now)` returns the entry (deadline `None`). This guards upgrade compatibility: if deserialization failed, `new_with_default` would silently replace the file with an empty default and the assertion would catch it.

Uploader (`upload/uploader.rs`), using the advancing test clock unless noted:

- `failed_upload_moves_on_to_next_job` (rewrites `failing_round_requeues_at_tail_behind_later_job`): script Hang(A), Ok(B), Ok(A); enqueue A, release the hang with an error while B is queued; assert `recorded_calls() == [A, B, A]` — exactly one attempt on A before B runs, then A's deferred retry.
- `retry_backoff_follows_expected_sequence` (rework): pin `attempts: 5` and backoff base 1/growth 2/max 4; script four `Err` then `Ok`; assert the recorded sleep durations are `[1, 2, 4, 4]` — waits grow across requeues (no per-round reset) and cap at `max_secs`.
- `attempt_cap_drops_job` (rewrites `global_attempt_cap_drops_job`): pin `attempts: 3`; three `Err` for A then `Ok` for B. The mock script is positional, so enqueue B only after awaiting three `started_rx` signals for A (as the existing test does) — otherwise B's attempt would consume one of A's scripted `Err`s. Assert A appears exactly 3 times then B, and the actor stays healthy.
- `waiting_job_is_skipped_and_enqueue_wakes_idle_wait` (new; frozen clock, never-completing `sleep_fn` of `std::future::pending`): A fails once and enters backoff; enqueue B; assert B is attempted while A is still waiting (`recorded_calls() == [A, B]`, `len() == 1`), proving both the skip and that a command interrupts the idle wait; shutdown must still return promptly.
- `requeue_into_full_queue_drops_job` (adjust): capacity 1; Hang(A) then Ok(B); B fills the queue while A is in flight; releasing A with an error makes its stamped requeue hit the full queue and drop; assert `recorded_calls() == [A, B]`.
- Unchanged in intent, updated only for the spawn signature: `processes_enqueued_job`, `terminal_failure_drops_job_without_requeue`, `shutdown_during_in_flight_upload_returns_promptly`, `shutdown_during_backoff_sleep_returns_promptly` (now exercises the idle wait), the post-shutdown error tests, `worker_exits_*`, `arc_handle_delegates_to_uploader`, `len_reports_queued_jobs`.

Avoid 4+ `assert_eq!` on fields of one variable in a single test (the import linter's field-by-field assert check).

Acceptance:

- `./scripts/test.sh` passes with zero failures; the new tests above fail before their milestone's change and pass after (in particular `waiting_job_is_skipped_and_enqueue_wakes_idle_wait` deadlocks/hot-loops on the old worker, and `failed_upload_moves_on_to_next_job` fails under the old in-place loop because A's second in-place attempt consumes B's positional `Ok` step and succeeds, recording `[A, A]`).
- `grep -rn "in_place_attempts\|max_total_attempts" agent/` returns no hits under `agent/src` or `agent/tests`; `UploaderOptions` exposes exactly `queue_capacity`, `attempts`, `backoff`; `UploaderOptions::default()` has `attempts == 30` and `backoff.max_secs == 3600` (assert these in a small options test).
- `./scripts/covgate.sh` and `./scripts/lint.sh` pass.
- **Preflight must report CLEAN — meaning `./scripts/preflight.sh` passes locally and CI is green on the pushed head of `fix/upload-queue-backoff` — before the PR leaves draft or the task is reported complete.**

## Idempotence and Recovery

Every step is re-runnable: edits are plain source changes, tests/lints are read-only, and commits are per-milestone so `git revert <milestone-commit>` (or `git reset --hard` before pushing) cleanly unwinds a milestone. The queue snapshot format change is forward-safe by construction (`#[serde(default)]` accepts old files; the legacy-snapshot test proves it) and needs no migration; if a device downgrades, older agents ignore the unknown `next_attempt_at` field because serde ignores unknown JSON keys by default. If milestone 2 goes wrong after milestone 1 is committed, the tree still builds and behaves identically to `main` (deadlines are all `None`), so it is safe to pause between milestones.
