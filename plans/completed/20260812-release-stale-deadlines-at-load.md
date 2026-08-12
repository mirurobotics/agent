# Rework: release stale upload retry deadlines at load instead of clamping on every pop

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (repo root `/home/ben/miru/workbench2/repos/agent`, crate `miru-agent` under `agent/`) | read-write | All edits: `agent/src/upload/queue.rs`, `agent/src/upload/uploader.rs`, `agent/tests/upload/queue.rs`, plus this plan file and a supersede note on `plans/completed/20260812-clamp-stale-retry-deadlines.md` |

This plan lives in the agent repo because every edit lands there. No backend, spec, or generated-code (`libs/`) changes. No wire-protocol, config, or on-disk-format changes.

This is a **rework of work already on a branch**: branch `fix/clamp-stale-retry-deadlines`, base `main`, with pull request **#202 already open and marked ready for review**. The rework lands as new commits pushed onto the same branch with a normal (non-force) push. PR #202 is updated, never closed and replaced.

## Purpose / Big Picture

The bug being fixed is unchanged from the original branch. `QueueEntry.next_attempt_at` is an absolute wall-clock instant that is persisted to the on-disk queue snapshot and trusted with no upper bound when read back. If the device clock steps backward — a robot with no battery-backed real-time clock boots at 1970, or NTP applies a large backward correction — every persisted deadline lands far in the future relative to the new `now`. `Queue::pop_ready` then never selects those entries, `Worker::run` parks in a multi-decade sleep, and because `attempts` only increments when an attempt actually runs, the 30-attempt drop cap never fires. Uploads stop permanently and the only recovery is deleting the snapshot file on the device by hand.

The branch currently fixes this by adding a `max_wait: TimeDelta` parameter to `Queue::pop_ready` and treating any deadline beyond `now + max_wait` as eligible immediately. After this rework, the recovery instead happens **once, at startup, on the load path**: a new `Queue::release_stale_deadlines(horizon)` clears any `next_attempt_at` beyond the horizon, and `pop_ready` goes back to its simple single-argument form.

Observable behavior after the rework: an agent that starts with a snapshot containing an entry stamped `now + 48h`, with a maximum backoff of one hour, releases that deadline to `None` at startup, logs a `WARN` naming how many deadlines were released, persists the corrected snapshot, and uploads that entry on the next loop iteration instead of stalling forever.

## Progress

- [ ] M1: `pop_ready` reverted to one argument; `Queue::release_stale_deadlines` added; `Worker::run` calls it once before the loop; idle-wait ceiling kept with a rewritten comment; three doc-comment fixes preserved. Commit.
- [ ] M2: `agent/tests/upload/queue.rs` — `horizon()` helper and the second `pop_ready` argument removed everywhere; the two horizon tests replaced by `release_stale_deadlines` tests. Commit.
- [ ] M3: Preflight CLEAN locally, new commits pushed onto `fix/clamp-stale-retry-deadlines`, PR #202 updated, CI green on the pushed branch head. Commit plan updates.

## Surprises & Discoveries

(Add entries as work proceeds.)

## Decision Log

- Decision: move the stale-deadline recovery off `pop_ready` and onto a dedicated method called once at startup.
  Rationale: threading a horizon argument through `pop_ready` pollutes a hot-path signature — 13 test call sites plus a `horizon()` test helper — for what is purely a defensive check. The dominant real-world trigger is a **reboot** (deadlines stamped at a correct wall-clock time, device reboots with no RTC, comes up at 1970, reloads the snapshot), which necessarily goes through the load path. The recovery therefore belongs on the load path, not on every pop.
  Date/Author: 2026-08-12 / ben@miruml.com.

- Decision: accept a narrower fix. A mid-run backward clock step with **no reboot** is no longer caught at pop time.
  Rationale: see "Known limitation" below. Reboot is the dominant trigger, and the bounded idle wait still guarantees eventual recovery once the clock is corrected forward.
  Date/Author: 2026-08-12 / ben@miruml.com.

- Decision: this plan supersedes the approach in `plans/completed/20260812-clamp-stale-retry-deadlines.md`. That plan stays in `plans/completed/` (it was executed) and gets a short supersede note.
  Rationale: the completed plan is an accurate record of what was built; rewriting it would erase history. A pointer is enough.
  Date/Author: 2026-08-12 / ben@miruml.com.

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

The upload path is an actor. The `Uploader` handle and the `Worker` run loop live in `agent/src/upload/uploader.rs`; the `Queue` lives in `agent/src/upload/queue.rs`; the tests are `agent/tests/upload/queue.rs` and `agent/tests/upload/uploader.rs`.

`QueueEntry` is `{ job, attempts, next_attempt_at }`. `next_attempt_at: Option<DateTime<Utc>>` is an **absolute wall-clock instant** meaning "earliest instant this entry may be popped"; `None` means "eligible now". The whole queue including these stamps is serialized to a JSON snapshot on disk (`QueueSnapshotFile`, an alias for `SingleThreadStateFile<QueueSnapshot, QueueSnapshot>`) on every mutation, and reloaded at startup by `Queue::from_snapshot`. A `Queue` built by `Queue::new` has no snapshot file; its private `persist()` is then a no-op.

Relevant `Queue` methods:

- `pop_ready(now, max_wait)` — currently two arguments on this branch; being reverted to `pop_ready(now)`.
- `earliest_next_attempt()` — minimum deadline over all entries, treating `None` as `DateTime::<Utc>::MIN_UTC`; returns `None` only when the queue is empty.
- `persist()` (private, async) — writes the current entries to the snapshot file, warns on failure.

`UploaderOptions.backoff` is a `cooldown::Backoff { base_secs, growth_factor, max_secs }`; the production default is `max_secs: 3600`. The maximum wait the retry schedule can ever produce is `max_secs` seconds, so a persisted deadline further out than `now + max_secs` cannot have been produced by that schedule.

Important structural fact, already verified: **`Uploader::spawn` is not `async`.** It builds the `Queue`, constructs the `Worker`, and calls `tokio::spawn(worker.run())`. An `async` recovery call therefore cannot live in `spawn` without adding an `.await` there; it belongs at the top of `Worker::run`, before the loop. This is the choice this plan makes.

### What is on the branch now (the thing being reworked)

`git diff origin/main...HEAD -- agent/` currently shows, across three files:

1. `agent/src/upload/queue.rs` — `pop_ready` gained a `max_wait: TimeDelta` parameter, computes `horizon = now.checked_add_signed(max_wait).unwrap_or(DateTime::<Utc>::MAX_UTC)`, and uses the predicate `at <= now || at > horizon`; its doc comment was rewritten to explain the horizon. Also `earliest_next_attempt`'s doc lost its trailing sentence "The worker uses it to size an idle sleep."
2. `agent/src/upload/uploader.rs` — `Worker::run` derives `let max_wait = TimeDelta::seconds(self.options.backoff.max_secs.max(0));` inside the loop and passes it to `pop_ready`; the `None`-with-non-empty-queue arm gained `let ceiling = max_wait.to_std().unwrap_or(Duration::ZERO);` and a `.min(ceiling)` on the computed wait, with a "defense-in-depth" comment. Also `idle_wait`'s doc comment was corrected.
3. `agent/tests/upload/queue.rs` — a `horizon()` helper returning `TimeDelta::hours(24)` was added, all 13 pre-existing `pop_ready` call sites gained `horizon()` as a second argument, and two tests were added: `deadline_beyond_horizon_is_eligible_now` and `deadline_at_horizon_is_still_waiting`.

Background on why the current shape exists: `plans/completed/20260812-clamp-stale-retry-deadlines.md`. Read it for context; its approach is superseded by this plan.

### Known limitation (state this plainly; do not imply full coverage)

This rework **narrows** the fix. Because the release now happens only once at startup, a backward clock step that occurs **mid-run, with no reboot**, is no longer caught: entries stamped before the step keep their far-future deadlines and stay unpopped. The only mitigation in that case is the bounded idle wait — the worker keeps waking at most one maximum-backoff apart, so the queue recovers as soon as the clock is corrected forward again, but until then those entries do not upload. This is a known, accepted trade: reboot is the dominant real-world trigger, and the pop-time check cost a hot-path signature and 13 test call sites to cover the rarer case.

A structurally correct long-term fix — persisting retry deadlines as **relative durations or monotonic offsets** rather than absolute wall-clock instants, which would make the whole class of clock anomalies inexpressible — is a possible follow-up. It is **not** implemented here.

### Out of scope

Do not implement or plan for any of these:

- Queue capacity/eviction policy versus the roughly 21-hour retry residency.
- Backoff jitter.
- Unifying the `sleep_fn` / `now_fn` seams into a `Clock` trait.
- The remaining upload test-coverage gaps (elapsed-time-aware idle sleep, exact backoff assertion on the timeout path, mid-queue-pop persistence) — those belong to a separate planned PR.
- Changing the persisted representation to relative/monotonic durations.

## Plan of Work

### M1 — source rework (`agent/src/upload/queue.rs`, `agent/src/upload/uploader.rs`)

**1. Revert `Queue::pop_ready` to its single-argument form.** Restore exactly the shape on `origin/main`:

    /// Remove and return the first entry whose `next_attempt_at` is `None` or
    /// `<= now`, preserving the order of the remaining entries. Returns `None`
    /// (without persisting) when no entry is eligible.
    pub async fn pop_ready(&mut self, now: DateTime<Utc>) -> Option<QueueEntry> {
        let idx = self
            .jobs
            .iter()
            .position(|entry| entry.next_attempt_at.is_none_or(|at| at <= now))?;
        let entry = self.jobs.remove(idx);
        if entry.is_some() {
            self.persist().await;
            info!("upload: job dequeued; queue length {}", self.jobs.len());
        }
        entry
    }

(Verify against `git show origin/main:agent/src/upload/queue.rs` rather than trusting this transcription.)

**2. Add `Queue::release_stale_deadlines`** immediately after `pop_ready` in `agent/src/upload/queue.rs`:

    pub async fn release_stale_deadlines(&mut self, horizon: DateTime<Utc>)

Behavior: iterate the entries; for each whose `next_attempt_at` is `Some(at)` with `at > horizon` (strictly beyond), set it to `None` and count it. If the count is zero, return without persisting — the method must not rewrite the snapshot needlessly. If the count is non-zero, `warn!` naming the count, then `await self.persist()`.

Suggested log text (adjust to match surrounding style — other messages in this file are prefixed `upload:`):

    warn!(
        "upload: released {released} retry deadline(s) beyond {horizon}; \
         the device clock appears to have stepped backward"
    );

The doc comment must explain the reasoning, not just the mechanics: a deadline beyond the caller's maximum backoff cannot have been produced by that backoff schedule, so it is evidence of a backward clock step — an unset real-time clock at boot, or a large NTP correction applied after the deadline was persisted. Clearing it makes the entry eligible now rather than stranded forever. Note that this is called once at startup, and that it warns because a clock anomaly is worth surfacing in device logs.

`TimeDelta` may no longer be needed in `queue.rs` — the new method takes an absolute `DateTime<Utc>` horizon, computed by the caller. Drop `TimeDelta` from the `chrono` import if unused; `cargo clippy` with `-D warnings` will catch it either way.

**3. Call it exactly once at startup.** In `agent/src/upload/uploader.rs`, at the **top of `Worker::run`, before the `loop`**:

    let horizon = (self.now_fn)() + TimeDelta::seconds(self.options.backoff.max_secs.max(0));
    self.queue.release_stale_deadlines(horizon).await;

`Uploader::spawn` is not `async`, so the call cannot live there without restructuring; `Worker::run` is the correct home. `.max(0)` guards a negative configured `max_secs` from producing a horizon in the past (which would release every deadline).

**4. Keep the idle-wait ceiling, replace its comment.** In the `None`-with-non-empty-queue arm of `Worker::run`, keep the `ceiling` local and the `.min(ceiling)`. Because `max_wait` is no longer needed for the pop, derive the ceiling directly in that arm from `self.options.backoff.max_secs`. Replace the "defense-in-depth" comment with the ceiling's actual job:

    // without a ceiling the worker can commit to a decades-long sleep that a
    // later forward clock correction would never interrupt; bounding the wait
    // makes it re-evaluate at least once per maximum backoff, so the queue
    // recovers once the clock is corrected

**5. Keep the three doc-comment fixes already on the branch, unchanged:**

- `earliest_next_attempt` in `queue.rs` no longer claims "The worker uses it to size an idle sleep."
- `idle_wait` in `uploader.rs` opens with "Sleep for `wait`, staying responsive to commands."
- The false sentence claiming the sleep completing counts as a command is gone, while the "Deliberately NOT [`Self::run_until_shutdown`]" rationale paragraph is preserved **verbatim**.

### M2 — tests (`agent/tests/upload/queue.rs`)

Delete the `horizon()` helper and its doc comment. Remove the second argument from every `pop_ready` call site. The `digests()` helper reverts to `queue.pop_ready(Utc::now()).await`.

Delete `deadline_beyond_horizon_is_eligible_now` and `deadline_at_horizon_is_still_waiting` from `mod pop_ready`.

Add a new `mod release_stale_deadlines` (sibling of `mod pop_ready`, following the same `use super::*;` convention) with three tests:

1. `stale_deadline_is_released_and_survives_reload` — build a disk-backed queue via `Queue::from_snapshot(8, open(&path).await)`; `requeue` an entry stamped `now + TimeDelta::hours(48)`; call `release_stale_deadlines(now + TimeDelta::hours(24))`; assert the entry is now immediately poppable via `pop_ready(now)` with `next_attempt_at == None` and its `attempts` preserved. In a separate scope, reopen the same path with `Queue::from_snapshot` and assert the reloaded entry also has `next_attempt_at == None` — proving the release was persisted.
2. `deadline_within_horizon_is_untouched` — an entry stamped exactly `now + TimeDelta::hours(24)` with a horizon of `now + TimeDelta::hours(24)` is left alone: `pop_ready(now)` returns `None` and `queue.len() == 1`. This pins the boundary as **strictly beyond**.
3. `no_stale_deadline_does_not_persist` — with a disk-backed queue holding only entries inside the horizon, call `release_stale_deadlines` and assert the snapshot was not rewritten. The cheapest observable proof available: capture the snapshot file's modified time (or its raw bytes) before and after the call and assert it is unchanged. Use whatever read helper the surrounding tests already use for the file; if reading raw bytes is awkward, mtime comparison is acceptable, but note in the test comment why the assertion is indirect. If neither is practical without new test infrastructure, drop this test rather than adding a production-code hook, and record that in Surprises & Discoveries.

Note when writing test 1 that `make_job` stamps a fresh `Utc::now()` on each call, so whole-`Job` equality does not survive a reload; assert on `job.digest` like the existing tests do.

The existing `agent/tests/upload/uploader.rs` tests do not use snapshot files and are expected to need **no** changes. The startup `release_stale_deadlines` line in `Worker::run` is executed by every uploader test, so it is covered.

### M3 — validate, push, update PR #202

Run the full local validation, push the new commits onto the existing branch, confirm CI is green on the pushed head, and add a one-line supersede note to `plans/completed/20260812-clamp-stale-retry-deadlines.md` pointing at this plan.

## Concrete Steps

**All commands run from `/home/ben/miru/workbench2/repos/agent`** so that `rust-toolchain.toml` (Rust 1.97.0) applies. Running cargo from a parent directory resolves 1.94.0 and fails on the AWS SDK MSRV.

Confirm the starting point:

    cd /home/ben/miru/workbench2/repos/agent
    git rev-parse --abbrev-ref HEAD     # expect: fix/clamp-stale-retry-deadlines
    git status --short                  # expect: clean

### M1

Edit `agent/src/upload/queue.rs` and `agent/src/upload/uploader.rs` per Plan of Work steps 1-5. Then:

    cargo fmt -p miru-agent -- --check
    cargo clippy --package miru-agent --all-features -- -D warnings

Expect no output from `fmt` and a clean clippy. Note: use `-p miru-agent`, never `--all`.

The queue tests will not compile until M2, which is expected. To check the source compiles on its own:

    cargo check --package miru-agent --features test

Commit M1:

    git add agent/src/upload/queue.rs agent/src/upload/uploader.rs
    git commit -m "refactor(upload): release stale retry deadlines at load instead of on every pop"

### M2

Edit `agent/tests/upload/queue.rs` per Plan of Work M2. Then:

    grep -n "horizon" agent/tests/upload/queue.rs     # expect: no matches
    cargo test --features test upload::

Expect a summary line of the form `test result: ok. NN passed; 0 failed; ...`. **Assert on "0 failed" plus the presence of the named new tests, never on a hard total** — the count moves with `main`. Confirm the new tests ran:

    cargo test --features test upload::queue::release_stale_deadlines -- --nocapture

Commit M2:

    git add agent/tests/upload/queue.rs
    git commit -m "test(upload): cover release_stale_deadlines on the load path"

### M3

Full local gate:

    ./scripts/lint.sh
    ./scripts/test.sh
    ./scripts/covgate.sh
    ./scripts/preflight.sh

`covgate.sh` enforces `agent/src/upload/.covgate`, which requires **96.00**. `preflight.sh` must print its clean result (the previous run of this branch printed `Preflight clean`).

Add the supersede note to `plans/completed/20260812-clamp-stale-retry-deadlines.md` — one line at the end of Outcomes & Retrospective, e.g. "Superseded: the `pop_ready` horizon argument was reverted in favour of a load-path `Queue::release_stale_deadlines`; see the release-stale-deadlines-at-load plan."

Push and verify CI:

    git push                       # normal push, NOT --force
    gh pr view 202
    gh pr checks 202

Expect PR #202 to still be the same PR, ready for review, now showing the new commits, with `lint`, `test`, and `tools` all green on the pushed head.

## Validation and Acceptance

Acceptance is behavioral, checked in this order:

1. **Signature reverted.** `grep -n "pop_ready" agent/src/upload/queue.rs agent/src/upload/uploader.rs agent/tests/upload/queue.rs` shows every call site passing exactly one argument, and `grep -n "horizon" agent/tests/upload/queue.rs` returns nothing.
2. **Stale deadline released.** `stale_deadline_is_released_and_survives_reload` passes: an entry stamped `now + 48h` with a `now + 24h` horizon ends with `next_attempt_at == None`, is returned by `pop_ready(now)`, and the reloaded snapshot also shows `None`. This test fails before the change (the method does not exist) and passes after.
3. **Boundary is strictly-beyond.** `deadline_within_horizon_is_untouched` passes: an entry stamped exactly at the horizon is still waiting, `queue.len() == 1`.
4. **No needless write.** `no_stale_deadline_does_not_persist` passes, or its omission is recorded in Surprises & Discoveries with the reason.
5. **Called once at startup.** `Worker::run` contains exactly one `release_stale_deadlines` call, before the `loop`, and `Uploader::spawn` contains none.
6. **Ceiling kept, comment replaced.** The `None`-with-non-empty-queue arm still has `.min(ceiling)`; `grep -n "defense-in-depth" agent/src/upload/uploader.rs` returns nothing; the replacement comment explains the decades-long-sleep rationale.
7. **Doc fixes preserved.** `earliest_next_attempt`'s doc has no "The worker uses it to size an idle sleep." sentence; `idle_wait`'s doc opens with "Sleep for `wait`, staying responsive to commands." and the "Deliberately NOT [`Self::run_until_shutdown`]" paragraph is intact verbatim.
8. **Suite green.** `cargo test --features test upload::` reports `0 failed`, and the named new tests appear in the run.
9. **Gates green.** `cargo fmt -p miru-agent -- --check` clean; `./scripts/lint.sh` exit 0 (a pre-existing unrelated `RUSTSEC` allowed warning is acceptable); `./scripts/covgate.sh` shows `agent/src/upload` at or above 96.00.

**Preflight must report CLEAN and CI must be green on the pushed branch head before this task is reported complete.** Concretely: `./scripts/preflight.sh` prints its clean result locally, and `gh pr checks 202` shows all checks passing on the head commit that `git push` created. A clean local preflight with red or pending CI is not acceptance.

## Idempotence and Recovery

Every step here is a source edit plus a local command; all are safe to repeat. `cargo fmt`, `clippy`, `test.sh`, `covgate.sh`, and `preflight.sh` are read-only with respect to source and can be rerun freely.

If M1 or M2 goes wrong mid-edit, `git checkout -- agent/src/upload/ agent/tests/upload/` restores the last commit; the branch history up to that point is intact. To recover the exact pre-rework text of any hunk, `git show 854bcaa:<path>` and `git show origin/main:<path>` both remain available.

The one step that is not locally reversible is `git push`. It is a normal push of new commits onto an existing branch, so it neither rewrites history nor disturbs PR #202. **Do not use `--force`** — a force push on a ready-for-review PR discards review anchors. If a pushed commit turns out to be wrong, fix it forward with an additional commit and push again.

The two source files each hold one logical change, so M1 and M2 can be reverted independently with `git revert <sha>` if the rework needs to be abandoned; that would restore the horizon-argument approach exactly as PR #202 currently has it.
