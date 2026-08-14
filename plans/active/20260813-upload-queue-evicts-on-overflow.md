# Evict the worst upload-queue entry on overflow instead of dropping the newly scanned file

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (this repo, checked out at `/home/ben/miru/workbench2/repos/agent`) | read-write | All source and test changes: the upload queue's admission policy, the requeue contract, the upload error surface, and the unit/integration tests that pin them. |

Branch: `fix/upload-queue-evicts-on-overflow`, already created off `main` at commit `3db7921`.

## Purpose / Big Picture

Today a device that has been offline long enough to fill its 1024-slot upload queue **silently loses every newly recorded file**. The scanner emits each stable file exactly once; if `Queue::enqueue` rejects it because the queue is full, nothing ever re-offers it, and the only trace is a single `warn!` line from the sink. The data is on disk but the agent has decided, permanently and quietly, never to upload it.

After this change the queue never refuses a new file. When it is at capacity it **evicts** one existing entry and admits the new job in its place, and it logs the eviction at `error!` with the evicted file, its attempt count, and its age. Loss becomes a recorded decision instead of an inference from a file that never showed up in the backend.

**Say this plainly, including in the PR body.** The eviction policy is "highest `attempts` first, ties broken by oldest `job.first_observed_at`". During the outage this feature exists for, **every entry has `attempts == 0`**: PR #185 exempted network-classified failures from the attempt budget, and during an outage every failure is network-classified. So `attempts` is a useless discriminator in exactly the scenario that matters, and the policy degenerates to its tie-break: **evict the oldest `first_observed_at`, i.e. keep the newest N files and drop the oldest**. The user has explicitly confirmed that newest-wins is the correct trade for this data. Do not bury this in a comment; it is the actual product behavior.

Observable outcomes after the change:

- With `queue_capacity: 1` and job A queued, enqueuing job B leaves the queue holding exactly B, length 1, and an `error!` line naming A.
- The eviction survives a process restart: the persisted snapshot on disk contains B and not A.
- A file evicted *while it is being uploaded* stays evicted — the worker's subsequent requeue does not bring it back.

## Progress

- [ ] M1: Queue evicts on overflow; `enqueue` becomes infallible; `requeue` refuses absent ids; `QueueFullErr` removed; workspace compiles.
- [ ] M2: Queue-level tests for admission, victim selection on both discriminators, persistence, and the `capacity == 0` / `capacity == 1` edges.
- [ ] M3: Uploader-level regression tests that an evicted in-flight entry is not resurrected, in memory and on disk.
- [ ] M4: Comparator perturbation check, coverage gate, full preflight, docs/comment sweep.

## Surprises & Discoveries

(Add entries as work proceeds.)

## Decision Log

(Add entries as work proceeds. The pre-authoring decisions that shaped this plan are recorded under "Plan of Work" -> "Decisions taken during authoring"; copy any that change during implementation down here with the new rationale.)

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

### Where the code lives

All paths are relative to `/home/ben/miru/workbench2/repos/agent`.

- `agent/src/data_uploads/upload/queue.rs` — the queue itself. `Queue` wraps a `VecDeque<QueueEntry>` plus a `capacity: usize` and an optional `QueueSnapshotFile` (an atomically-written JSON snapshot on disk).
- `agent/src/data_uploads/upload/uploader.rs` — the actor. `Worker` owns the `Queue` and drives uploads; `Uploader` is the handle that sends `Command`s over an mpsc channel. `UploaderOptions::queue_capacity` defaults to `1024`, `attempts` to `30`, backoff caps at 3600s.
- `agent/src/data_uploads/upload/sink.rs` — `UploadStableFileSink::on_stable_file`, the scanner's callback. Around line 56 it does `if let Err(e) = self.uploader.enqueue(job).await { warn!(...) }`. This is the site where a rejected file is lost.
- `agent/src/data_uploads/upload/job.rs` — `Job`, with `file`, `size`, `digest`, `mtime`, `first_observed_at`, `last_observed_at`, `file_rule_id`, `deployment_id`, `retention`.
- `agent/src/data_uploads/upload/errors.rs` — `QueueFullErr` (fields `capacity`, `file`, `trace`) and the `UploadErr` enum, wired to the shared error trait by `crate::impl_error!`.
- `agent/tests/data_uploads/upload/queue.rs` — queue tests. This is an *integration* test crate, so it can only use `miru_agent`'s public API.
- `agent/tests/data_uploads/upload/uploader.rs` — actor tests, including `mod durability` at the bottom, the only place that observes the persisted snapshot while an attempt is in flight.

### Terms

- **Entry vs job.** A `Job` is the file to upload. A `QueueEntry` is `{ id: Uuid, job: Job, attempts: u32, next_attempt_at: Option<DateTime<Utc>> }`.
- **In flight.** `Queue::next_ready(now)` returns a **clone** of the first eligible entry and **deliberately leaves it in the queue** — and therefore in the persisted snapshot. This is what makes the queue at-least-once (PR #204). The entry leaves storage only when the worker calls `Queue::remove(id)` or hands it to `Queue::requeue(entry)`. **So the job currently being uploaded is still an ordinary queue entry.**
- **Counted vs network failure.** `Worker::handle_counted_failure` increments `entry.attempts` and drops the job at the budget. `Worker::handle_network_failure` does **not** increment `attempts` (PR #185); it only drops on the `max_job_age` backstop.
- **Scanner emit-once.** The scanner offers each stable file to the sink exactly once. There is no re-offer, so a sink-side failure is permanent loss.

### The bug, precisely

`Queue::enqueue` calls the private `verify_capacity`, which returns `Err(UploadErr::QueueFullErr(..))` when `self.jobs.len() >= self.capacity`. The new job is never pushed. `sink.rs` logs a `warn!` and returns. Combined with the 30-attempt / ~21h retry residency and the network-error exemption, a long outage fills all 1024 slots with old, indefinitely-retrying entries, and every file scanned thereafter is dropped.

## Plan of Work

### Decisions taken during authoring

**Evict one entry per enqueue, not "evict down to capacity".** `Queue::from_snapshot` deliberately loads an over-capacity backlog in full (e.g. after someone lowers `queue_capacity`). A `while len >= capacity` loop would mass-evict on the next enqueue. A single `if len >= capacity { evict_one() }` bounds the work per enqueue, keeps the queue's length non-increasing, and preserves the existing "an over-capacity backlog drains rather than grows" intent.

**Handle `capacity == 0` and `capacity == 1` by evicting strictly *before* the push, and by making eviction a no-op on an empty queue.** With `capacity == 1` the single incumbent is the victim and the new job takes its place — length stays 1. With `capacity == 0` the condition `0 >= 0` holds on every enqueue; on an empty queue there is no victim, eviction returns `None`, and the job is admitted, so the queue degenerates to capacity 1 and can never spin or evict the job it just admitted.

**Trap 1 — the in-flight entry is an eviction candidate. Resolution: option (b), `requeue` refuses to re-add an id the queue no longer holds.**

The hazard: the worker holds a *clone* from `next_ready` while the entry is still in the queue. During the attempt, `Worker::run_until_shutdown` keeps serving commands, so an `Enqueue` can arrive mid-upload and evict that very entry. When the attempt then fails, `Worker::handle_counted_failure` / `handle_network_failure` -> `requeue_after` -> `Worker::requeue` -> `Queue::requeue(entry)`, which today does `remove_impl(entry.id)` (a no-op — it is gone) then `push_back` — **resurrecting the evicted entry and pushing the queue back over capacity**, silently defeating the eviction.

Chosen: `Queue::requeue` looks up `entry.id`; if it is not present the requeue is a **no-op** and reports that, so the queue never grows through `requeue`. `Queue::enqueue` becomes the single growth path and therefore the single place capacity is enforced — a structural invariant rather than a discipline the caller must remember. This matches the direction of PR #206 ("make durability structural").

- Rejected (a) *never evict the in-flight id.* Requires the queue to track the worker's phase, coupling two components that are currently independent, and it does not close the hole: with `capacity == 1` and the sole entry in flight there is no eligible victim, so `enqueue` would have to reject again — reintroducing exactly the bug this plan fixes, at the edge.
- Rejected (c) *worker checks membership before requeueing.* Correct (the actor is single-threaded, so there is no TOCTOU) and it costs zero test churn, but the invariant then lives in the caller: any future requeue site that forgets the check reintroduces the resurrection. Keep the membership *reporting* from (b) so the worker can still log, but let the queue enforce it.

**Contract change and its fan-out.** `Queue::requeue`'s contract changes from "replace or append" to "move an entry the queue still holds to the tail; if it is no longer queued, do nothing". The only production caller is `Worker::requeue`, which always passes an entry obtained from `next_ready`. The fan-out is in tests: `agent/tests/data_uploads/upload/queue.rs` uses `requeue` with a **fresh `Uuid::new_v4()`** as a back door to seed entries carrying a chosen `attempts` and `next_attempt_at`. Those seeds become no-ops and are converted to a `seed_entry` helper.

**Trap 2 — the error surface. `verify_capacity` is `enqueue`'s only error source**, so `enqueue` becomes infallible, mirroring what PR #200 did for `requeue`. `UploadErr::QueueFullErr` then has no construction site anywhere in `agent/src`. A repo-wide grep confirms the only remaining references are the upload tests that assert it, and a *separate, unrelated* `QueueFullErr` under `agent/src/data_uploads/retention/` which must be left alone. Fan-out is therefore small: delete the struct, its `UploadErr` variant, and its `impl_error!` entry.

**Eviction logging needs a clock; take `now` as a parameter.** `Queue` holds no clock — `next_ready(now)` already takes one, and `Worker` injects `now_fn` so tests can freeze time. `Queue::enqueue` gains a `now: DateTime<Utc>` parameter used solely to compute the evicted entry's age for the log line. Rejected: calling `Utc::now()` inside `queue.rs` (breaks the injected-clock discipline the test helpers rely on).

### Edits, file by file

**`agent/src/data_uploads/upload/queue.rs`**

1. Change the signature to `pub async fn enqueue(&mut self, job: Job, now: DateTime<Utc>)` — no `Result`. Body: `self.make_room(now);` then the existing `push_back` of a fresh `QueueEntry`, then `self.persist().await`, then the existing `info!`. Update the doc comment to describe eviction, and state the `attempts == 0` degeneracy in one sentence.
2. Delete `verify_capacity` entirely. Add `make_room(&mut self, now: DateTime<Utc>)`: return early if `self.jobs.len() < self.capacity`; otherwise pick the victim index, remove it, and log at `error!` with the file, rule, digest, attempts, and `(now - first_observed_at).num_minutes()`, stating plainly that the file will never be uploaded. Add a `victim_index(&self) -> Option<usize>` using

        .max_by_key(|(_, entry)| (entry.attempts, std::cmp::Reverse(entry.job.first_observed_at)))

   `std::cmp::Reverse` inverts the secondary key so the maximum is the *oldest* `first_observed_at`. Add `error` to the existing `tracing` import.
3. Change `pub async fn requeue(&mut self, entry: QueueEntry) -> bool`. Find the index of `entry.id`; if absent, return `false` immediately (no push, **no persist**). Otherwise remove at that index, `push_back(entry)`, `persist().await`, log the existing `info!`, return `true`. Document the contract: returns `false` when the entry is no longer queued — confirmed, dropped, or evicted while the caller held its clone — and note that `enqueue` is the only path that grows the queue.
4. Update the `from_snapshot` doc comment: an over-capacity backlog is no longer only drained by uploads; each enqueue evicts one entry, so the queue never grows past its loaded length.

**`agent/src/data_uploads/upload/uploader.rs`**

5. `Command::Enqueue`'s `respond_to` becomes `oneshot::Sender<()>`.
6. In `handle_command`, the `Command::Enqueue` arm computes `let now = (self.now_fn)();` and dispatches `self.queue.enqueue(job, now).await`.
7. `Worker::requeue` consumes the new `bool`; when it is `false`, `warn!` that the file was evicted while uploading and its requeue is being dropped. Capture `file` and `attempts` before the move.
8. `UploaderExt::enqueue` keeps its `Result<(), UploadErr>` signature — channel send/receive errors remain — but the body loses its trailing `?`.
9. Update the `UploaderOptions::queue_capacity` doc comment: at capacity, a new job evicts an existing one rather than being refused.

**`agent/src/data_uploads/upload/errors.rs`**

10. Delete the `QueueFullErr` struct and its `impl crate::errors::Error`, the `UploadErr::QueueFullErr` variant, and the `QueueFullErr` line in `crate::impl_error!(UploadErr { .. })`. Leave `agent/src/data_uploads/retention/errors.rs::QueueFullErr` untouched — a different type for a different queue.

**`agent/src/data_uploads/upload/sink.rs`**

11. Code unchanged (the `enqueue` call can still fail on actor transport errors). Update the struct doc comment: overflow no longer rejects — the queue evicts and logs at `error!` — and the remaining failure mode is the actor being gone.

## Concrete Steps

Every command runs from `/home/ben/miru/workbench2/repos/agent`. **Do not run cargo from a parent directory**: `rust-toolchain.toml` pins 1.97.0 here, and a parent-directory invocation resolves 1.94.0 and fails on the AWS SDK MSRV.

### M1 — Source change and mechanical test fixes

1. Apply edits 1-11 from Plan of Work.

2. Fix the test files that no longer compile. In `agent/tests/data_uploads/upload/queue.rs`:

   - Add a `now` argument to every `queue.enqueue(..)` call and drop `.unwrap()`.
   - Add helpers next to `make_job`: `make_job_observed_at(name, first_observed_at)` and

         /// Seed an entry carrying `attempts` and `next_attempt_at`. `requeue` only
         /// moves an entry the queue already holds, so enqueue first and requeue the
         /// entry the queue handed back.
         async fn seed_entry(queue: &mut Queue, job: Job, attempts: u32,
                             next_attempt_at: Option<DateTime<Utc>>) -> Uuid

     Implement it as: enqueue, `next_ready(DateTime::<Utc>::MAX_UTC)` (required — earlier seeds may carry future deadlines that `next_ready` would skip), then `requeue` the entry with the desired fields, asserting the requeue returned `true`. Seed each entry immediately after enqueuing it, never in a batch.
   - Convert every `requeue`-as-seed call site to `seed_entry`: `requeue::preserves_attempts_and_appends_at_tail`, `requeue::persists_attempts`, `requeue::next_attempt_at_survives_reload`, `next_ready::skips_waiting_entries`, `next_ready::returns_none_when_all_waiting`, `earliest_next_attempt::returns_min_deadline`, `earliest_next_attempt::none_deadline_counts_as_min_utc`, `reset_invalid_deadlines::only_deadlines_past_the_horizon_move`, `reset_invalid_deadlines::reset_is_in_memory_until_the_next_mutation`. Raise any capacity that a seed would now push over.
   - Delete `enqueue::full_queue_returns_queue_full_err`, `enqueue::rejection_does_not_persist`, and `requeue::full_queue_still_accepts_requeue` (all replaced in M2), and drop the now-unused `UploadErr` import if nothing else uses it.
   - Add `requeue::requeue_of_an_absent_entry_is_a_no_op`: requeue a `QueueEntry` with a fresh `Uuid` into an empty `Queue::new(4)`; assert it returns `false` and the queue is still empty.

   In `agent/tests/data_uploads/upload/uploader.rs`: delete `in_flight_job_holds_its_capacity_slot` (it asserts `QueueFullErr`); it is replaced in M3. Deleting here and adding there keeps each commit compiling.

3. Verify:

        cargo build --package miru-agent --features test
        cargo test --features test data_uploads::upload

   Expect `0 failed`. Read the named tests, never a hard suite total.

4. Commit:

        git add agent/src agent/tests
        git commit -m "fix(upload): evict the worst queue entry on overflow instead of dropping the new file"

### M2 — Queue-level tests

5. Add to `mod enqueue` in `agent/tests/data_uploads/upload/queue.rs`:

   - `full_queue_evicts_and_admits_the_new_job`: `Queue::new(2)` holding `a.log`, `b.log`; enqueue `c.log`; assert `len() == 2` and the survivors are `b.log`, `c.log`.
   - `evicts_the_most_failed_entry_first`: an older 0-attempt entry and a newer 4-attempt entry; enqueue a third; the 4-attempt entry is evicted. Pins the primary discriminator.
   - `among_equal_attempts_the_oldest_is_evicted`: two equal-attempts entries at `t0` and `t0 + 1h`; enqueue a third; the `t0` entry goes. Pins the tie-break — the case that actually fires during an outage.
   - `eviction_survives_a_reload`: over a snapshot-backed `Queue::from_snapshot(1, ..)`, enqueue `a.log` then `b.log`; reload from the same path; only `b.log` remains. Replaces the deleted `rejection_does_not_persist`.
   - `capacity_one_keeps_only_the_newest`.
   - `capacity_zero_admits_one_entry_and_does_not_spin`: `Queue::new(0)`; enqueue twice; `len() == 1` and the survivor is the second job. The test completing at all is the non-spin assertion.
   - `requeue::full_queue_still_accepts_a_requeue_of_a_queued_entry`: `Queue::new(1)`; enqueue; take via `next_ready`; requeue with changed attempts; assert it returned `true` and `len() == 1`. Requeue must not be blocked by a full queue and must not grow it.

6. Verify and commit:

        cargo test --features test data_uploads::upload::queue
        git add agent/tests
        git commit -m "test(upload): pin overflow eviction, victim selection, and the capacity edges"

### M3 — Uploader-level regression tests (the important ones)

7. In `mod durability`, generalize the spawn helper to `spawn_persisted_with_options(mock, snapshot, options)`, built exactly like the existing `spawn_persisted_with_test_clock` but taking `options`; have the existing helper delegate with the defaults. **Use the test-clock variant, not `spawn_persisted`.** `spawn_persisted` pairs a no-op sleep with a real `Utc::now`; if the resurrection bug is present, the resurrected entry carries a backoff stamp and the run loop busy-loops in `idle_wait`, so the regression would manifest as a **hung test binary** instead of a failed assertion.

8. Add `in_flight_job_evicted_by_a_new_enqueue_is_not_resurrected`:
   - `MockStep::Hang(release_rx)` then `MockStep::Ok`; spawn with `UploaderOptions { queue_capacity: 1, ..Default::default() }` over a temp snapshot path.
   - Enqueue A; await its start — A is in flight and still queued.
   - Enqueue B — served mid-attempt; it evicts A and admits B.
   - Assert immediately: `on_disk(&path).await == ["sha256:b.log"]` and `uploader.len() == 1`.
   - Release A's attempt with `scripted_err()`; the worker requeues, which must be a no-op.
   - Await the next start; it must be B.
   - `await_drained`, then `on_disk(&path).await.is_empty()`.
   - Shut down; assert `mock.recorded_calls() == vec![job_a, job_b]` — exactly two attempts. Under the resurrection bug this is `[A, B, A]`.

9. Add a lighter in-memory companion using `spawn_with_test_clock` with `queue_capacity: 1`, asserting `uploader.len()` stays 1 across the eviction and `recorded_calls()` is `[A, B]`.

10. Verify and commit:

        cargo test --features test data_uploads::upload::uploader
        git add agent/tests
        git commit -m "test(upload): prove an evicted in-flight job is not resurrected by its requeue"

### M4 — Perturbation, coverage, preflight

11. **Perturb the comparator and confirm the selection tests fail.** First make sure the working tree is clean — the revert below uses `git checkout --`, and doing that over uncommitted work destroys it. That mistake has already cost work in this workstream.

    Perturbation A — in `victim_index`, change `max_by_key` to `min_by_key`. Run `cargo test --features test data_uploads::upload::queue`; expect `evicts_the_most_failed_entry_first` to FAIL.

    Perturbation B — restore `max_by_key` and drop the `std::cmp::Reverse` wrapper. Expect `among_equal_attempts_the_oldest_is_evicted` and `full_queue_evicts_and_admits_the_new_job` to FAIL.

    Restore with `git checkout -- agent/src/data_uploads/upload/queue.rs` after each and re-run to green. Record the observed failures in Surprises & Discoveries.

12. Confirm no stale references and no damage to the retention error type:

        grep -rn "QueueFullErr" --include=*.rs agent/src agent/tests

    Expect hits **only** under the retention module and its tests/mocks. Zero hits under `agent/src/data_uploads/upload/` or `agent/tests/data_uploads/upload/`.

13. Coverage and formatting:

        cargo fmt -p miru-agent -- --check
        ./scripts/covgate.sh

    `agent/src/data_uploads/upload/.covgate` requires `96.00`. Never run `cargo fmt --all`.

14. Full preflight:

        ./scripts/preflight.sh

    Known flake: preflight sometimes reports one failing component under its own parallelism while every component passes when run individually. If that happens, re-run the reported component alone and record the result.

15. Push and open the PR as a draft. **The PR body must state the `attempts == 0` degeneracy in its own paragraph**, and must record the `requeue` contract change and the removal of `UploadErr::QueueFullErr`.

## Validation and Acceptance

Run from `/home/ben/miru/workbench2/repos/agent`:

    cargo test --features test data_uploads::upload

Interpret by the `0 failed` count and the named tests below — never by a hard total.

1. **New files are never refused.** `enqueue::full_queue_evicts_and_admits_the_new_job`.
2. **Victim selection is pinned on both discriminators.** `enqueue::evicts_the_most_failed_entry_first` and `enqueue::among_equal_attempts_the_oldest_is_evicted`. Both must FAIL under the M4 perturbations and pass when restored.
3. **The eviction is durable.** `enqueue::eviction_survives_a_reload`.
4. **Edges behave.** `enqueue::capacity_one_keeps_only_the_newest` and `enqueue::capacity_zero_admits_one_entry_and_does_not_spin` — the latter passing at all (rather than hanging) is the assertion.
5. **The in-flight entry is not resurrected — the headline regression test.** `durability::in_flight_job_evicted_by_a_new_enqueue_is_not_resurrected`, plus its in-memory companion.
6. **`requeue` no longer grows the queue.** `requeue::requeue_of_an_absent_entry_is_a_no_op` and `requeue::full_queue_still_accepts_a_requeue_of_a_queued_entry`.
7. **The error surface is clean.** The M4 grep returns zero `QueueFullErr` hits under the upload module, and the retention module's own `QueueFullErr` is untouched (`cargo test --features test data_uploads::retention` still passes).
8. **Coverage holds.** `./scripts/covgate.sh` passes at or above `96.00`.
9. **Preflight is CLEAN before the PR leaves draft.** `./scripts/preflight.sh` clean locally AND CI green on the pushed branch head. The only exception is the documented preflight parallelism flake, acceptable only when the reported component passes on its own and that is recorded in Surprises & Discoveries.

Optional log sanity check:

    RUST_LOG=info cargo test --features test data_uploads::upload::queue::enqueue::capacity_one_keeps_only_the_newest -- --nocapture

Expect an `ERROR` line naming the evicted file, its attempt count, and its age.

## Idempotence and Recovery

- All `cargo`, `grep`, and script steps are read-only or rebuild-in-place and can be re-run freely.
- The source edits are ordinary code changes on a dedicated branch; `git diff` and `git checkout --` recover any file.
- **The one genuinely risky step is the M4 perturbation.** Reverting it discards *all* uncommitted changes in that file. Commit or stash before perturbing — M4 step 11 requires a clean tree first, and this is not optional: destroying uncommitted work this way has already happened once in this workstream.
- No data migration and no on-disk format change: `QueueEntry` and `QueueSnapshot` are untouched, so snapshots load unchanged in both directions. Rollback is a plain revert of the branch.
- If M1's test conversions get tangled, `git checkout -- agent/tests/data_uploads/upload/queue.rs` restores the original and the conversion can be redone one `mod` at a time.
