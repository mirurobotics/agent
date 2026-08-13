# Refactor: make the retention delete queue's durability structural, not incidental

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (repo root `/home/ben/miru/workbench3/repos/agent`, crate `miru-agent` under `agent/`) | read-write | `agent/src/data_uploads/retention/{queue.rs,deleter.rs,mod.rs}`, `agent/tests/data_uploads/retention/{mod.rs,queue.rs}` |

Base branch `main`; working branch `refactor/deleter-queue-identity-removal` (already created, currently no commits ahead of `main`). No backend, spec, or generated-code (`libs/`) changes. No wire-protocol or config changes. `agent/src/data_uploads/retention/.covgate` (98.39) must be satisfied, never edited.

## Purpose / Big Picture

**This is a refactor, not a bug fix. The Deleter is correct on `main` and nothing is losing data today.**

Verified on `main`:

- `Queue::pop_front` / `Queue::requeue` touch memory only; `Queue::persist` is separate and caller-driven.
- `SingleThreadDeleter::sweep` (`agent/src/data_uploads/retention/deleter.rs:95`) pops an entry into ownership, does the stat/hash/delete work, and persists only once the entry has resolved (`deleter.rs:107`). Nothing persists between the pop and the resolution, so a crash mid-sweep leaves the entry on disk and it is retried.
- `SingleThreadDeleter` owns `Queue` by value — no `Arc`, no `Mutex`. Exactly two production persist sites: `deleter.rs:86` (enqueue) and `deleter.rs:107` (sweep).
- `Worker::run` is `while let Some(cmd) = self.receiver.recv().await` and awaits `sweep()` to completion inside its match arm, so no `Enqueue`/`Shutdown` can interleave with a sweep.
- The Deleter is not wired into the application. `agent/src/data_uploads/mod.rs` declares the module; nothing outside `retention/` constructs a `Deleter` (the only external references are `agent/tests/data_uploads/retention/deleter.rs` and the test mock `agent/tests/mocks/deleter.rs`).

The problem worth fixing is that this correctness rests on a **non-local argument** — "the run loop never services a command during a sweep" — rather than on structure. Wiring the Deleter in will require a sweep ticker, and a sweep that stats and hashes files runs long, so whoever wires it will make it responsive to shutdown via `select!` over the receiver, exactly as `agent/src/data_uploads/upload/uploader.rs`'s `run_until_shutdown` and `idle_wait` already do. At that moment `enqueue`'s `persist()` can fire between the `pop_front` and the resolution, and the snapshot written by that enqueue will not contain the popped entry. The entry is then gone from disk while it exists only in a stack frame — silent loss, introduced by an unrelated-looking change to the run loop.

After this change the queue is at-least-once by construction: an entry is never absent from the deque while it is being worked, so no persist from any source can write it out of the snapshot. The ticker work can then proceed without a durability argument attached to it.

The reference implementation is the upload queue after `5a6cae2` ("fix(upload): keep a job queued until its upload is confirmed", #204). Mirror its structure and naming.

## Progress

- [ ] M1: `Queue` gains entry identity, `next_ready`, `remove`; `persist` becomes private and every mutator persists.
- [ ] M2: `SingleThreadDeleter::sweep` becomes select-and-resolve; `SweepOutcome::NotDue` disappears; inline deleter tests updated.
- [ ] M3: Queue tests move to `agent/tests/data_uploads/retention/queue.rs`, mirroring `agent/tests/data_uploads/upload/queue.rs`, plus the mid-sweep-persist durability test.
- [ ] M4: Preflight CLEAN locally, pushed, draft PR opened, CI green on branch head before the PR leaves draft.

## Surprises & Discoveries

_(fill in during execution)_

## Decision Log

**D1 — Entries get a `Uuid` id; no existing `Job` field is a usable key.**

`Job` (`agent/src/data_uploads/retention/job.rs`) is `{ file, size, digest, mtime, first_observed_at, last_observed_at, ttl_secs, file_rule_id, deployment_id }`. There is no id. Candidate natural keys and why each fails:

- `file` alone: the queue deliberately admits duplicates for one path. Pinned by `queue.rs` `enqueue::duplicate_same_path_jobs_are_both_queued` and by `deleter.rs` `sweep::same_path_duplicate_resolves_in_one_sweep`, whose whole point is that two jobs for the same file coexist and the sweep resolves both in one pass (the second drops as already-gone).
- `file + digest`: the duplicate case above is precisely a same-path, same-digest pair (`make_job(tmp.file(), 900, 0)` and `make_job(tmp.file(), 1000, 0)` differ only in observation timestamps).
- `file + digest + last_observed_at`: nothing forbids re-enqueuing a job with identical fields — the producer is a scan sink re-reporting a stable file, and `Job` derives `Clone`. A key that is unique only by accident is worse than no key: `remove` would take the wrong entry.

So the uploader's reasoning applies unchanged here — duplicate jobs for the same file legitimately coexist — and the same answer applies: a per-entry `Uuid`, minted at enqueue, that is identity for the queue and meaningless to the job.

**D2 — The sweep loop: budget over *due* entries, select-and-resolve, requeue only on Retry.** _(the important one)_

On `main` the loop is `for _ in 0..n { pop_front; ...; requeue or persist }`, and rotation is what stops a not-due entry at the head from being re-selected forever. With peek semantics the entry is left in place, so the rotation no longer happens for free and the naive port re-selects the same not-due entry every iteration.

The fix has two halves:

1. **Readiness moves into the queue.** `Queue::next_ready(&self, now)` returns a clone of the first entry with `entry.job.due_at() <= now`, skipping not-due entries without moving them — the exact analogue of the uploader's `next_attempt_at` filter. `SweepOutcome::NotDue` is deleted, and `sweep_entry`'s leading `if now < entry.due_at()` check goes with it: an entry that is not due is simply never selected. Not-due entries are therefore never requeued and never persisted, which is strictly less disk churn than `main`.
2. **`Retry` still rotates.** A transient stat/hash/delete failure leaves the entry due, so leaving it in place would re-select it immediately. `Queue::requeue(entry)` (remove-by-id + `push_back`, then persist) moves it behind everything else. The entry is in the deque at every instant, including across the persist, so the rotation cannot lose it.

**Loop bound.** `now` is captured once per sweep and `Job::due_at()` is a pure function of immutable fields, so the set `D` of due entries is fixed for the pass. Every visit either removes the entry or pushes it to the very back — behind every not-yet-visited entry. Therefore, while any unvisited due entry remains, `next_ready` returns an unvisited one, and `|D|` iterations visit each due entry exactly once. The loop is:

```rust
let now = (self.now_fn)();
// Budget: one visit per entry that is due at `now`. A retried entry is
// requeued at the tail, behind every entry not yet visited, so this budget
// is exactly enough to visit each due entry once and never twice.
for _ in 0..self.queue.count_ready(now) {
    let Some(entry) = self.queue.next_ready(now) else {
        break;
    };
    match Self::sweep_entry(&entry.job, now).await {
        SweepOutcome::Retry => self.queue.requeue(entry).await,
        SweepOutcome::Deleted | SweepOutcome::AlreadyGone | SweepOutcome::Changed => {
            self.queue.remove(entry.id).await;
        }
    }
}
```

`count_ready(now)` is a new `Queue` method (`self.entries.iter().filter(|e| e.job.due_at() <= now).count()`).

Rejected: keeping `for _ in 0..self.queue.len()`. With peek semantics the budget must count *due* entries, not all entries. Counter-example: queue `[A(retry), W(not due)]`, `len() == 2`. Iteration 1 visits `A`, which fails transiently and is requeued → `[W, A]`. Iteration 2 calls `next_ready`, which skips `W` and returns `A` again — a second stat+hash of the same failing entry in one pass. This is not a correctness bug (the operations are idempotent and a second attempt that succeeds is fine), but it is duplicated syscalls and duplicated warn-level log lines, bounded by the not-due population. The precise budget costs one `O(n)` count per sweep.

Rejected: `while let Some(entry) = queue.next_ready(now)` with a `first_requeued: Option<Uuid>` sentinel to detect a full lap. Equally precise and `O(1)` state, but its *termination* depends on the same ordering argument that its correctness does — if the argument ever breaks, the failure mode is a hung worker instead of some redundant work. A counted loop cannot hang. Rejected: a `HashSet<Uuid>` of visited ids — same guarantee, more state and allocation per sweep.

**D3 — `len()`/`is_empty()`/capacity now include the entry being swept.**

The selected entry never leaves the deque, so `len()` counts it for the duration of its sweep step and it holds its capacity slot. This mirrors the uploader post-#204 (`UploaderExt::len`'s doc comment and `in_flight_job_holds_its_capacity_slot`). Two notes specific to retention:

- The capacity overshoot the upload change had to reason about **cannot occur here at all**. `requeue` is now a rotation of an entry that is already in the deque (remove-by-id then push_back), so it does not change `len()`. The queue holds at most `capacity` entries, full stop. The existing test `requeue::bypasses_capacity` is therefore no longer about bypassing capacity; it is rewritten as "a requeue at capacity rotates the entry and leaves `len()` unchanged".
- No existing *deleter* test changes behavior: all of them assert queue contents after `sweep()` returns, and by then every entry has resolved. Only the queue-level `pop_front`-shaped tests change, and those are being rewritten anyway (see M3).

`DeleterExt::len`'s doc comment gains the uploader's wording: the count includes an entry currently being swept, because it stays queued until it resolves.

**D4 — Snapshot: new nested `QueueEntry` shape, with `#[serde(default = "Uuid::new_v4")]` on `id`, and no migration for the old format.**

`DeleteQueueSnapshot.entries` changes from `Vec<Job>` to `Vec<QueueEntry>` where `QueueEntry { id: Uuid, job: Job }` — the uploader's shape and field names. An existing `delete_queue.json` written by `main` (a flat array of `Job` objects) will not deserialize; `SingleThreadStateFile::new_with_default` (`agent/src/filesys/state_file.rs:54`) falls back to `create(file, &default, Overwrite::Allow)` on any read/parse error, so such a file is silently overwritten with an empty snapshot rather than failing the boot. That is acceptable **only** because the module is unshipped and unwired: nothing constructs a `Deleter`, so no agent in the field has ever written a `delete_queue.json`. Record this explicitly in the PR body.

`#[serde(default = "Uuid::new_v4")]` on `id` is kept for exact symmetry with the uploader and so a hand-written snapshot need not carry ids — but be honest in the doc comment: unlike the uploader's, it buys no migration of the previous on-disk format, because the entry shape itself changed.

Rejected: `#[serde(flatten)] job` to preserve the flat on-disk shape and get true backward compatibility from the `id` default alone. It would work, but it diverges from the uploader's layout for a compatibility guarantee that nothing needs, and `flatten` brings its own costs (buffered deserialization, no `deny_unknown_fields`). Rejected: putting `id` on `Job` — identity is a queue concern, and `Job` is a value the producer constructs.

**D5 — Persistence unifies inside `Queue`; `Deleter` stops persisting.**

`Queue::persist` becomes private. `enqueue`, `remove`, and `requeue` each persist as their last act; `next_ready` and `count_ready` never persist and are not `async`. `SingleThreadDeleter::enqueue` loses its `self.queue.persist().await` line and becomes a thin `self.queue.enqueue(job).await`; `sweep` loses its `self.queue.persist().await` arm. After this there is exactly one persist implementation and no caller can forget to call it — which is the same structural move as the identity change, applied to the write path.

Failure direction is safe in every case, as it is for the uploader: a swallowed persist failure on `remove` leaves a resolved entry on disk, which the next sweep drops as `AlreadyGone`; a swallowed failure on `requeue` leaves the entry in its pre-rotation position, which is retried. Every swallowed failure fails toward redundant work, never toward loss.

## Context and Orientation

Retention is an actor: `Deleter` (handle) + `Worker` (loop) over `SingleThreadDeleter` in `agent/src/data_uploads/retention/deleter.rs`, holding a `Queue` from `agent/src/data_uploads/retention/queue.rs`. `Job::due_at()` (`job.rs:24`) is `last_observed_at + ttl_secs`, saturating to `DateTime::<Utc>::MAX_UTC` on overflow, i.e. "never due".

The reference implementation to mirror, method for method:

- `agent/src/data_uploads/upload/queue.rs` — `QueueEntry` with `#[serde(default = "Uuid::new_v4")] id: Uuid`; `next_ready(&self, now)` returning a clone and leaving the entry in the deque; `remove(&mut self, id) -> Option<QueueEntry>` as the only removal-persisting path, with a private `remove_impl`; `requeue(entry)` doing `remove_impl(entry.id)` then `push_back`.
- `agent/src/data_uploads/upload/uploader.rs` — the worker hands the id back at each resolution point (`uploader.rs:200,217,243,248`).
- `agent/tests/data_uploads/upload/queue.rs` and `agent/tests/data_uploads/upload/uploader.rs` (`mod durability`) — the test structure to mirror.

Test layout differs between the two modules today and this plan converges it. The upload queue has **no** inline tests; everything lives in `agent/tests/data_uploads/upload/queue.rs` against the public API (`upload/mod.rs` re-exports `Queue, QueueEntry, QueueSnapshot, QueueSnapshotFile`). The retention queue has ~270 lines of inline `#[cfg(test)] mod tests` plus a `#[cfg(test)] pub(crate) fn entries()` accessor, and `agent/tests/data_uploads/retention/` holds only three actor-boundary tests in `deleter.rs`.

The `entries()` accessor stays: the inline `deleter.rs` tests use it in roughly fifteen assertions and it is the cheapest way to keep them untouched. Change only its body so it still returns `Vec<Job>` (`self.entries.iter().map(|e| e.job.clone()).collect()`); every existing `assert_eq!(deleter.queue.entries(), [job])` then compiles and passes unchanged.

## Plan of Work

### `agent/src/data_uploads/retention/queue.rs`

Add `use uuid::Uuid;` to the external-crates group and `chrono::{DateTime, Utc}` (needed by `next_ready`/`count_ready` signatures). Follow the file's existing three-group import convention.

New type, above `DeleteQueueSnapshot`:

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueueEntry {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    pub job: Job,
}
```

`DeleteQueueSnapshot.entries` becomes `Vec<QueueEntry>`; `Queue.entries` becomes `VecDeque<QueueEntry>`.

- `enqueue(&mut self, job: Job) -> Result<(), DeleteErr>` becomes `async`. Same capacity check and `QueueFullErr` (reading `job.file` before the move), then `push_back(QueueEntry { id: Uuid::new_v4(), job })`, then `self.persist().await`, then the existing `info!`.
- `pop_front` is **deleted**.
- `next_ready(&self, now: DateTime<Utc>) -> Option<QueueEntry>` — `self.entries.iter().find(|e| e.job.due_at() <= now).cloned()`, with the uploader's doc comment adapted: the entry is deliberately left in the queue, and so in the persisted snapshot, until the caller passes its id to `remove` or hands the entry to `requeue`; that is what makes the queue at-least-once, and what makes a persist from any other source unable to write it out of the snapshot. Not `async`.
- `count_ready(&self, now: DateTime<Utc>) -> usize` — the sweep's budget (see D2). Doc-comment it as such so it does not read as an idle accessor.
- `remove(&mut self, id: Uuid) -> Option<QueueEntry>` and private `remove_impl(&mut self, id) -> Option<QueueEntry>` — copy the uploader's shape, including "the only point at which a job leaves durable storage" and "unknown ids are ignored".
- `requeue(&mut self, entry: QueueEntry)` — `self.remove_impl(entry.id); self.entries.push_back(entry); self.persist().await;`. Doc: this is a rotation of an entry that is already queued, not an admission, so it is not capacity-gated and cannot change `len()`.
- `persist` loses `pub`.
- `entries()` keeps its `#[cfg(test)] pub(crate)` gating; body maps to `e.job.clone()`.
- Delete the whole inline `#[cfg(test)] mod tests` block (moved in M3), along with the now-unused test-only imports.

### `agent/src/data_uploads/retention/mod.rs`

Extend the queue re-export to `pub use self::queue::{DeleteQueueSnapshot, DeleteQueueSnapshotFile, Queue, QueueEntry};`, matching `upload/mod.rs`. Required by the relocated tests. Do **not** rename `DeleteQueueSnapshot*` to the uploader's `QueueSnapshot*`; the retention module is `pub use`d under its own path and the rename is churn with no reader benefit (recorded in Out of Scope).

### `agent/src/data_uploads/retention/deleter.rs`

- Import `QueueEntry` alongside `Queue`; add `uuid` only if a signature needs it (it should not — ids flow inside `QueueEntry`).
- Delete `SweepOutcome::NotDue`.
- `sweep()` takes the shape in D2. Rewrite its doc comment to state the new invariant in structural terms: each entry is *selected*, not removed; it stays in the queue — and therefore in every snapshot written while it is being worked, by this sweep or by any concurrent command — until it resolves; a resolved entry is removed and persisted before the next is selected; a transient failure rotates it to the tail. Add the budget argument from D2 verbatim as the loop's comment.
- `sweep_entry(entry: &Job, now)` — drop the leading `if now < entry.due_at()` check and the `now` parameter if it becomes unused (it does: readiness was its only use; drop the parameter and the local `now` binding stays for `count_ready`/`next_ready`). Keep `stat_file`/`check_file_identity`/`check_digest_mismatch`/`delete_file` unchanged.
- `enqueue` becomes `self.queue.enqueue(job).await` (drop the separate `persist`).
- `DeleterExt::len` doc: note the count includes an entry currently being swept.
- Update the `Deleter` handle doc comment: it already says the driver imposes the sweep cadence; add that a sweep may now be interleaved with commands without risking the entry being swept, since the entry never leaves the queue.

### Inline tests in `deleter.rs` (`mod tests`)

Behavior is unchanged, so nearly all of them stand as written thanks to the `entries()` shim. Expected edits:

- `sweep::each_drop_is_persisted_before_the_next_job` — still valid; the drop now persists via `Queue::remove`. Update its comment to reference `remove` rather than the sweep's persist call.
- `enqueue::persist_failure_is_swallowed` — still valid; the persist now happens inside `Queue::enqueue`.
- Add `sweep::not_due_entry_is_left_in_place_and_not_persisted`: with a snapshot file, enqueue one entry with a TTL in the future, sweep, and assert the on-disk entry count and order are unchanged and the entry keeps its id — pinning that a not-due entry is no longer rotated.

### `agent/tests/data_uploads/retention/mod.rs`

Add `mod queue;` alongside `mod deleter;`.

## Concrete Steps

Run every command from `/home/ben/miru/workbench3/repos/agent`. This matters: `rust-toolchain.toml` pins the toolchain, and invoking cargo from a parent directory resolves a different version and fails on the AWS SDK MSRV.

### M1 — Queue restructure

1. Edit `agent/src/data_uploads/retention/queue.rs` and `mod.rs` per Plan of Work, including deleting the inline `mod tests` (M3 re-homes it).
2. `cargo check --package miru-agent` — fails until M2 (`deleter.rs` still calls `pop_front`/`persist`). Expected; do not chase it.

### M2 — Sweep restructure

3. Edit `agent/src/data_uploads/retention/deleter.rs` per Plan of Work.
4. `cargo check --package miru-agent` — clean.
5. `./scripts/test.sh` filtered: `RUST_LOG=off cargo test --features test data_uploads::retention` — the inline deleter tests must pass with no behavioral edits beyond those listed. Any deleter test that needs a *behavioral* change is a signal the refactor changed semantics: stop and re-read D2 before editing the test.
6. Commit: `refactor(retention): keep a delete job queued until its sweep resolves it`.

### M3 — Tests

7. Create `agent/tests/data_uploads/retention/queue.rs`, structured module-for-module on `agent/tests/data_uploads/upload/queue.rs`. Helpers at the top: `make_job(name, observed_secs, ttl_secs)` (port the deterministic one from the inline tests — fixed size/digest/mtime so whole-struct equality survives a reload), `open(path) -> DeleteQueueSnapshotFile`, a `drain(&mut queue) -> Vec<String>` that mirrors upload's `digests` helper (`while let Some(entry) = queue.next_ready(now) { out.push(...); queue.remove(entry.id).await; }` — draining without removing would return the same entry forever), and `on_disk(path) -> Vec<String>` reading through a fresh handle. Modules and cases:
   - `from_snapshot`: `empty_snapshot_loads_empty_queue`; `raw_json_snapshot_loads` (port from the inline tests, updated to the nested `{"id":...,"job":{...}}` shape — this is the test that pins the persisted wire format); `entry_without_id_gets_one` (raw JSON omitting `id`, asserting it loads and the entry is removable by the id it was given).
   - `enqueue`: `appends_in_fifo_order`; `duplicate_same_path_jobs_are_both_queued` (two *identical* jobs, asserting the two entries have distinct ids — this is D1's justification, pinned); `persists` (inverts the old `does_not_persist`); `full_queue_returns_queue_full_err`; `rejection_does_not_persist`; `persist_failure_is_swallowed` (port the directory-in-place-of-file trick).
   - `next_ready`: `returns_due_entries_in_fifo_order`; `skips_not_due_entries` (a not-due head does not hide a due entry behind it); `returns_none_when_nothing_is_due`; `leaves_the_entry_on_disk_until_removed`.
   - `count_ready`: `counts_only_due_entries`; `zero_when_empty`.
   - `remove`: `removes_the_entry_from_disk`; `unknown_id_is_ignored`; `removing_one_duplicate_leaves_the_other`.
   - `requeue`: `moves_the_entry_to_the_tail`; `at_capacity_rotates_without_growing` (replaces the old `bypasses_capacity`: capacity 1, one entry, `next_ready` then `requeue` → `len() == 1`); `survives_reload`.
   - `durability` — **the point of the refactor**: `persist_during_an_in_flight_entry_keeps_it_on_disk`. Over a real temp-dir snapshot: enqueue `a.log` and `b.log`; select `a` with `next_ready` and hold it (this is the sweep's in-flight state); then `enqueue("c.log")`, whose persist is the write that a `select!`-driven run loop would perform mid-sweep; assert `on_disk(&path)` contains `a`, `b`, and `c` — the in-flight `a` was not written out of the snapshot. Then `remove(a.id)` and assert disk holds exactly `b`, `c`. Comment it with why it is simulated at the `Queue` level: the actor cannot currently interleave a command with a sweep, so the interleaving is performed directly on the queue; the test exists so that making the run loop responsive to shutdown later cannot silently reintroduce the loss.
     - Verification that it is a real regression test (scratch only, do **not** commit): on `main`, the equivalent sequence is `pop_front()` followed by `enqueue()` (which on this branch persists) — the popped entry is absent from the resulting snapshot and the assertion fails. Confirm this by hand before trusting the test, since the new test cannot compile against `main`'s API and so cannot be run there directly.
8. Add `mod queue;` to `agent/tests/data_uploads/retention/mod.rs`.
9. `./scripts/test.sh` — full suite; expect `0 failed`. Do not assert a total test count; it ages badly against a moving `main`. Confirm by name that every module listed in step 7 ran.
10. Commit: `test(retention): pin queue identity, selection, and mid-sweep durability`.

### M4 — Preflight, push, CI, PR

11. `cargo fmt -p miru-agent -- --check` (never `--all`) and `./scripts/lint.sh` — both clean. The import linter enforces the three-group ordering in every touched file, and the `--assert-paths agent/tests` pass flags 4+ field-by-field `assert_eq!` in one test function; keep the new queue tests asserting on drained vectors rather than field-by-field, or add `// lint:allow(field-by-field-assert)` with a reason.
12. `./scripts/covgate.sh` — `agent/src/data_uploads/retention/.covgate` requires 98.39. **Never edit a `.covgate` file.** If the gate misses, the shortfall is an untested new branch (most likely `remove`'s unknown-id path or `next_ready`'s `None` path); add the test.
13. `./scripts/preflight.sh` — must print `Preflight clean`.
14. Push `refactor/deleter-queue-identity-removal`; open a DRAFT PR onto `main` with the full body at creation time (`gh pr create`). The body must lead with "this is a refactor; the Deleter is correct on `main`" and carry the D2 loop argument and the D4 snapshot note.
15. Watch CI on the pushed branch head; take the PR out of draft only per Validation and Acceptance below.

## Validation and Acceptance

Behavioral acceptance criteria:

1. **The structural guarantee**: `durability::persist_during_an_in_flight_entry_keeps_it_on_disk` in `agent/tests/data_uploads/retention/queue.rs`. A persist landing between selection and resolution leaves the selected entry in the snapshot. The `main`-shaped equivalent (`pop_front` + a persisting `enqueue`) loses the entry; verify that by hand per step 7 before relying on the test.
2. **No sweep semantics changed**: every existing test in `deleter.rs`'s inline `mod sweep` passes with no behavioral edit — specifically `due_entry_behind_a_not_due_entry_is_still_swept`, `same_path_duplicate_resolves_in_one_sweep`, `positive_delay_entry_waits_for_due_at`, `stat_failure_retains_entry`, `hash_failure_retains_entry`, `delete_failure_retains_entry`, and `each_drop_is_persisted_before_the_next_job`.
3. **One persist implementation**: `grep -n "persist" agent/src/data_uploads/retention/*.rs` shows the definition plus call sites only inside `enqueue`, `remove`, and `requeue` in `queue.rs` — no hit in `deleter.rs`.
4. **No pop-shaped API survives**: `grep -rn "pop_front" agent/src/data_uploads/retention agent/tests/data_uploads/retention` returns nothing.
5. **Identity is per entry, not per job**: `enqueue::duplicate_same_path_jobs_are_both_queued` asserts two identical `Job`s produce two entries with distinct ids, and `remove::removing_one_duplicate_leaves_the_other` asserts they are independently removable.

Exact commands (from `/home/ben/miru/workbench3/repos/agent`) and expected results:

    RUST_LOG=off cargo test --features test data_uploads::retention   # 0 failed
    ./scripts/test.sh                                                 # 0 failed
    cargo fmt -p miru-agent -- --check                                # exit 0, no diff
    ./scripts/lint.sh                                                 # exit 0
    ./scripts/covgate.sh                                              # retention gate 98.39 met
    ./scripts/preflight.sh                                            # prints "Preflight clean", exit 0

**Gate: `./scripts/preflight.sh` must report CLEAN locally and CI must be green on the pushed branch head before the PR leaves draft and before this task is reported complete.** Do not mark this work complete on a red or pending CI run.

## Idempotence and Recovery

Every step is an ordinary edit or a read-only check on an existing branch; all are safe to repeat. Revert uncommitted work with `git checkout -- agent/src/data_uploads/retention agent/tests/data_uploads/retention`; revert committed work with `git reset --hard <last-good-sha>`; reset the branch entirely with `git reset --hard main`. `cargo check`, `cargo test`, `lint.sh`, `covgate.sh`, and `preflight.sh` have no side effects beyond `target/` and temp dirs. Force-push is acceptable before review starts. Nothing here touches `libs/backend-api`, `libs/device-api`, or `api/specs/` — a diff there is a mistake; revert it. Never edit a `.covgate` file; a failing gate is a missing test, not a threshold to lower.

## Out of Scope

- Wiring the Deleter into `AppState` and giving it a sweep ticker. That is the change this refactor exists to de-risk; it lands separately.
- Making `Worker::run` responsive to shutdown via `select!`. Same reason — safe to do *after* this, and the durability test is what keeps it safe.
- A `Queue::earliest_due_at()` analogue of the uploader's `earliest_next_attempt`, for sizing an idle sleep. There is no sleep to size until the ticker exists.
- Renaming `DeleteQueueSnapshot`/`DeleteQueueSnapshotFile` to the uploader's `QueueSnapshot`/`QueueSnapshotFile`.
- Any per-entry attempt budget or backoff for retention (`SweepOutcome::Retry` retries forever today). A retention entry whose file cannot be stat'd is retried on every sweep indefinitely; that is `main`'s behavior and it stays.
- Deduplicating same-path jobs at enqueue. The sweep resolves duplicates in one pass, and D1 depends on duplicates being legal.

## Outcomes & Retrospective

_(fill in at completion)_
