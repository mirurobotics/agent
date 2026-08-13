# Fix: `Queue::requeue` must not evict an already-admitted upload job

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (repo root `/home/ben/miru/workbench2/repos/agent`, crate `miru-agent` under `agent/`) | read-write | The whole change: `agent/src/upload/queue.rs`, `agent/src/upload/uploader.rs`, `agent/tests/upload/queue.rs`, `agent/tests/upload/uploader.rs` |

This plan lives in `plans/active/20260812-requeue-bypasses-capacity.md` in the agent repo because every edit lands in that repo. Base branch `main`; working branch `fix/requeue-bypasses-capacity` (already created). No backend, spec, or generated-code (`libs/`) changes. No wire-protocol or config changes.

## Purpose / Big Picture

Today an upload job that has already been admitted to the queue can be silently and permanently dropped after a single failed attempt, because a job that arrived while it was in flight took its slot. After this change, a job that has been admitted is retried under the normal retry budget (default `attempts: 30`, `agent/src/upload/uploader.rs`) regardless of what arrived meanwhile. Observable difference: with `queue_capacity: 1`, upload A in flight, upload B enqueued mid-attempt, A's attempt failing — the executor sees calls `[A, B, A]` instead of `[A, B]`.

The loss is permanent, not merely delayed: the scanner emits each stable file exactly once per observation, so a dropped upload job is never re-created until the file's metadata changes.

## Progress

- [x] M1: `Queue::requeue` becomes infallible; `Worker::requeue` loses its dead error branch. (`26bb694`)
- [x] M2: Invert the two tests that asserted the drop behavior; drop the now-invalid `.unwrap()`s on other `requeue` call sites. (`afff53b`)
- [x] M3: Preflight CLEAN locally (`Preflight clean`), pushed, draft PR opened, CI watched on branch head.

## Surprises & Discoveries

- `cargo test --features test upload::` reports **73 passed**, not the 69 the plan predicted. The plan's count was stale relative to `main` (the upload suite grew after the count was taken); no test was lost or skipped, and both inverted tests are present and passing. Treat 73 as the correct baseline.
- The `upload` coverage gate rose to 96.61% against its 96.00 minimum, as the plan anticipated — removing the untested error branch is net-positive for coverage.
- `./scripts/lint.sh` reports a pre-existing `RUSTSEC-2026-0253` advisory (`lru 0.16.4` via `aws-sdk-s3`) as an allowed warning. Unrelated to this change; the script still exits clean.

## Decision Log

- Decision: `queue_capacity` is a **soft** bound — the queue may transiently hold `capacity + 1` entries. `Queue::requeue` performs no capacity check; only `Queue::enqueue` does.
  Rationale: a requeue is a *restore* of an entry that was already admitted, not a new admission. The overshoot is exactly one entry because the uploader actor is single-threaded and runs one attempt at a time, so at most one popped entry is outstanding. New enqueues are still rejected at capacity, so the bound on *admissions* is unchanged.
  Rejected alternative: reserve the in-flight slot (gate `enqueue` at `capacity - 1` while an attempt is running). That requires new in-flight-tracking state on `Queue` and couples the queue to the worker's execution phase, to buy a hard bound that nothing in the codebase depends on.
  Date/Author: 2026-08-12 / ben@miruml.com.

## Outcomes & Retrospective

Shipped as planned, in two commits, with no deviation from the Plan of Work:

- `26bb694` — `fix(upload): make requeue infallible so an in-flight job is never evicted` (`agent/src/upload/queue.rs`, `agent/src/upload/uploader.rs`).
- `afff53b` — `test(upload): assert a requeue past capacity retains the job` (`agent/tests/upload/queue.rs`, `agent/tests/upload/uploader.rs`).

All four acceptance criteria hold: `requeue_into_full_queue_retains_job` records `[a, b, a]`; `full_queue_still_accepts_requeue` gives `len() == 2` in FIFO order; `verify_capacity` is referenced only by its definition and the `enqueue` call site; `Worker::requeue` has no error arm and no `entry.job.clone()`.

Validation: 73 upload tests pass, `cargo fmt -p miru-agent -- --check` clean, `./scripts/lint.sh` exit 0, `./scripts/covgate.sh` upload 96.61% vs 96.00 required, `./scripts/preflight.sh` prints `Preflight clean`.

Retrospective note: the one friction point was the stale expected test count in Concrete Steps. Expected-count assertions in plans age badly against a moving `main`; a plan is better off asserting "0 failed" plus the named new tests than a total.

## Context and Orientation

The upload path is an actor: `Uploader` (handle) and `Worker` (loop) in `agent/src/upload/uploader.rs`, over a `Queue` in `agent/src/upload/queue.rs`.

`Queue` wraps a `VecDeque<QueueEntry>` plus a `capacity: usize` and an optional JSON snapshot file. `QueueEntry` is `{ job, attempts, next_attempt_at }`. Relevant methods:

- `enqueue(job)` — calls the private `verify_capacity`, which returns `UploadErr::QueueFullErr` when `jobs.len() >= capacity`, then pushes a fresh entry (`attempts: 0`).
- `pop_ready(now)` — **removes** and returns the first entry eligible to run (`next_attempt_at` is `None` or `<= now`).
- `requeue(entry)` — pushes a previously popped entry back at the tail, preserving `attempts`.

The bug: on `main`, `requeue` calls `verify_capacity` exactly like `enqueue`. But `verify_capacity` counts only *queued* entries; the in-flight job is not among them, because `pop_ready` already removed it. The worker serves commands during an attempt (via `run_until_shutdown`), so an `Enqueue` arriving mid-upload is admitted into the slot the popped job vacated. When that attempt then fails, `requeue` finds the queue full, returns `QueueFullErr`, and `Worker::requeue` logs an `error!` and drops the job — after one attempt, with the 30-attempt retry budget never applied.

Test layout mirrors source: `agent/tests/upload/queue.rs` and `agent/tests/upload/uploader.rs`. The uploader tests have three spawn helpers (top of `agent/tests/upload/uploader.rs`): `spawn_uploader` (real `Utc::now` clock, no-op sleep), `spawn_with_test_clock` (shared clock that a recording `sleep_fn` advances by each requested duration, so backoff stamps clear deterministically), and `spawn_frozen`. Anything backoff-shaped must use `spawn_with_test_clock`; with a no-op sleep and a non-advancing clock a backed-off entry never becomes eligible and the worker busy-loops.

## Plan of Work

**`agent/src/upload/queue.rs`, `Queue::requeue`** — change the signature to `pub async fn requeue(&mut self, entry: QueueEntry)`: drop the `self.verify_capacity(&entry.job).await?` call, the `Result<(), UploadErr>` return, and the trailing `Ok(())`. Keep the `push_back` / `persist` / `info!` body. Extend the doc comment to say the omission is deliberate — the entry was admitted when first enqueued, so a job arriving while it is in flight must never evict it, and the queue therefore holds at most `capacity` entries plus the single in-flight one. `verify_capacity` stays, still used by `enqueue`.

**`agent/src/upload/uploader.rs`, `Worker::requeue`** — the `if let Err(requeue_err) = ... { error!(...) }` branch is now dead; replace it with a bare `self.queue.requeue(entry).await;`. The `let job = entry.job.clone();` existed only so the error log could read `job.file_rule_id` / `job.file` / `job.digest` after `entry` moved into `requeue`; borrow instead (`let file = &entry.job.file;`) and delete the clone. Trim the doc comment's "dropping it with a warning if the queue rejects it". The `error!` import stays — other call sites in the file use it.

**`agent/tests/upload/queue.rs`** — seven `requeue(...)` call sites in `mod requeue`, `mod pop_ready`, and `mod earliest_next_attempt` end in `.await.unwrap()`; the `.unwrap()` is now a type error, so drop it. Rewrite `requeue::full_queue_rejects_requeue` as `requeue::full_queue_still_accepts_requeue`: capacity 1, enqueue `a.log`, requeue an entry for `b.log`, then assert `queue.len() == 2` and that draining via the existing `digests` helper yields `["sha256:a.log", "sha256:b.log"]` — FIFO order preserved past capacity. The `UploadErr` import stays; `enqueue`'s full-queue test still uses it.

**`agent/tests/upload/uploader.rs`** — rewrite `requeue_into_full_queue_drops_job` as `requeue_into_full_queue_retains_job`. Script the mock with `Hang(release_rx)`, `Ok`, `Ok` (a third step, for A's retry). Switch from the inline `Uploader::spawn(...)` real-clock construction to `let (uploader, handle, _sleeps) = spawn_with_test_clock(mock.clone(), options);` with `UploaderOptions { queue_capacity: 1, ..Default::default() }` — required, because A's backoff stamp must clear before its retry can run. Sequence: enqueue A, await its start, enqueue B (takes A's freed slot), release A's attempt with `scripted_err()`, await two more starts (B, then A), shut down. Assert `mock.recorded_calls() == vec![job_a.clone(), job_b, job_a]`.

## Concrete Steps

Run every command from `/home/ben/miru/workbench2/repos/agent`. This matters: `rust-toolchain.toml` pins 1.97.0, and invoking cargo from a parent directory resolves 1.94.0 and fails on the AWS SDK MSRV.

### M1 — Source fix

1. Edit `agent/src/upload/queue.rs` and `agent/src/upload/uploader.rs` per Plan of Work.
2. `cargo check --package miru-agent` — expect success. The test crates will not compile yet (`.unwrap()` on `()`); that is M2.
3. Commit: `fix(upload): make requeue infallible so an in-flight job is never evicted`.

### M2 — Tests

4. Edit `agent/tests/upload/queue.rs` and `agent/tests/upload/uploader.rs` per Plan of Work.
5. `cargo test --features test upload::` — expect 69 passed. The `--features test` flag is mandatory; many helpers and mocks are behind the `test` feature and omitting it fails with misleading missing-helper errors. Expected tail:

       test result: ok. 69 passed; 0 failed; 0 ignored

6. Commit: `test(upload): assert a requeue past capacity retains the job`.

### M3 — Preflight, push, CI, PR

7. `cargo fmt -p miru-agent -- --check` (never `--all`) and `./scripts/lint.sh` — both clean.
8. `./scripts/covgate.sh` — `agent/src/upload/.covgate` requires 96.00; expect it met. The change is net-negative in lines and removes an error branch that had no test, so coverage should rise.
9. `./scripts/preflight.sh` — must print `Preflight clean`.
10. Push `fix/requeue-bypasses-capacity`; open a DRAFT PR onto `main` with the full body at creation time (`gh pr create`).
11. Watch CI on the pushed branch head; take the PR out of draft only per Validation and Acceptance below.

## Validation and Acceptance

Behavioral acceptance criteria:

1. **The bug (uploader level)**: `requeue_into_full_queue_retains_job` in `agent/tests/upload/uploader.rs`. With `queue_capacity: 1`: A in flight, B enqueued mid-attempt into the slot A vacated, A's attempt fails. The mock executor's recorded calls are `[a, b, a]` — A is retried after B, not dropped. This test fails on `main` (recorded calls are `[a, b]`) and passes after the change.
2. **The bug (queue level)**: `requeue::full_queue_still_accepts_requeue` in `agent/tests/upload/queue.rs`. Capacity 1, one queued entry, one requeued entry → `len() == 2`, drain order `["sha256:a.log", "sha256:b.log"]`. Fails to compile/assert on `main`; passes after.
3. **Admission control unchanged**: the existing `enqueue` full-queue test still returns `UploadErr::QueueFullErr` at capacity. `verify_capacity` is still called from `enqueue` and only from `enqueue`:

       grep -n "verify_capacity" agent/src/upload/queue.rs   # expect: the enqueue call site and the definition only

4. **No dropped-job path remains in the requeue flow**: `Worker::requeue` in `agent/src/upload/uploader.rs` has no error arm and no `entry.job.clone()`.

Exact commands (from `/home/ben/miru/workbench2/repos/agent`) and expected results:

    cargo test --features test upload::   # 69 passed; 0 failed
    ./scripts/lint.sh                     # exit 0
    cargo fmt -p miru-agent -- --check    # exit 0, no diff
    ./scripts/covgate.sh                  # upload gate 96.00 met
    ./scripts/preflight.sh                # prints "Preflight clean", exit 0

**Gate: `./scripts/preflight.sh` must report CLEAN locally and CI must be green on the pushed branch head before the PR leaves draft.** Do not mark this work complete on a red or pending CI run.

## Idempotence and Recovery

Every step is an ordinary edit or a read-only check on an existing branch; all are safe to repeat. Revert uncommitted work with `git checkout -- agent/src/upload agent/tests/upload`; revert committed work with `git reset --hard <last-good-sha>`; reset the branch entirely with `git reset --hard main`. `cargo check`, `cargo test`, `lint.sh`, `covgate.sh`, and `preflight.sh` have no side effects beyond `target/`. Force-push is acceptable before review starts. Nothing here touches `libs/backend-api`, `libs/device-api`, or `api/specs/` — a diff there is a mistake; revert it.

## Out of Scope

Do not address these here; they are separate, known concerns:

- The queue's full-queue policy of rejecting new jobs rather than evicting stale failing ones (the capacity-vs-retry-residency issue: an entry can occupy a slot for ~21h across its 30-attempt budget).
- Backoff jitter.
- Unifying the sleep/clock injection seams across the uploader test helpers.
