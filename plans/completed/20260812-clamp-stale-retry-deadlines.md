# Fix: clamp stale upload retry deadlines so a backward clock step cannot stall the queue

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (repo root `/home/ben/miru/workbench2/repos/agent`, crate `miru-agent` under `agent/`) | read-write | The whole change: `agent/src/upload/queue.rs`, `agent/src/upload/uploader.rs`, `agent/tests/upload/queue.rs` |

This plan lives in `plans/active/20260812-clamp-stale-retry-deadlines.md` in the agent repo because every edit lands in that repo. Base branch `main` (at `e07b6f5`); working branch `fix/clamp-stale-retry-deadlines` (already created). No backend, spec, or generated-code (`libs/`) changes. No wire-protocol or config changes. The on-disk queue snapshot format is unchanged — this is a read-side interpretation change only, so snapshots written by older agents load and behave identically absent a clock anomaly.

## Purpose / Big Picture

Today, if the device clock steps backward — a robot with no battery-backed real-time clock boots at 1970, or NTP applies a large correction after the agent has already stamped retry deadlines — the upload queue stalls permanently. Every persisted `next_attempt_at` now sits far in the future relative to the new `now`, so no entry is ever eligible, the worker parks in a multi-decade sleep, the 30-attempt drop budget never advances (attempts only increment when an attempt actually runs), and queued entries occupy capacity forever. The only recovery is manually deleting the snapshot file on the device.

After this change, the queue self-heals: a deadline further in the future than the agent's own maximum backoff could ever have produced is treated as evidence of a clock anomaly, and the entry becomes eligible immediately. Observable difference: with an entry stamped `now + 48h` and a maximum backoff of 24h, `pop_ready` returns that entry instead of `None`.

This change also fixes three inaccurate doc comments in the same two files (see Plan of Work).

## Progress

- [x] M1: `Queue::pop_ready` takes a horizon; `Worker::run` derives one horizon local and uses it for both the pop and a ceiling on the idle wait; three doc-comment fixes. (`854bcaa`)
- [x] M2: Queue tests updated for the new argument; two new tests covering both sides of the horizon. (`5df33e3`)
- [x] M3: Preflight CLEAN locally, pushed, draft PR #202 opened, CI green on branch head.

## Surprises & Discoveries

- The upload suite is 75 tests on this branch (73 on `main` at `e07b6f5`, plus the two new ones) — as the plan anticipated, the count moves with `main`, so acceptance was checked on `0 failed` plus the two named tests rather than a total.
- No other change was needed: every pre-existing `pop_ready` call site took the new `horizon()` argument with all assertions intact, and no uploader test changed behavior — confirming the plan's read that nothing stamps a deadline past the default 3600s `max_secs`.

## Decision Log

- Decision: the horizon is passed to `Queue::pop_ready` as an argument, not stored on `Queue`.
  Rationale: `Queue` is a policy-free data structure — it holds a `VecDeque<QueueEntry>`, a capacity, and a snapshot file, and knows nothing about backoff. Giving it a `cooldown::Backoff` or `UploaderOptions` field would invert that. There is exactly one production call site (`Worker::run` in `agent/src/upload/uploader.rs`), so threading the value through is cheap, and tests can then pick a horizon independent of production defaults.
  Date/Author: 2026-08-12 / ben@miruml.com.

## Outcomes & Retrospective

Executed exactly as planned; no deviations from the Plan of Work. Draft PR
[#202](https://github.com/mirurobotics/agent/pull/202) onto `main`, three commits:

- `854bcaa` — `fix(upload): clamp retry deadlines beyond the max backoff so a backward clock step cannot stall the queue`
- `5df33e3` — `test(upload): cover both sides of the pop_ready staleness horizon`
- `31eb910` — `docs(plans): record M1/M2 progress and discoveries for the stale-deadline clamp`

All acceptance criteria in Validation and Acceptance were checked and hold: both
new tests pass and pin the clamp as strictly `>`; the pre-existing eligibility
tests pass unchanged apart from the new argument; `grep` confirms `queue.rs`
still has no `Backoff`/`UploaderOptions`/`cooldown` reference and no longer
mentions the worker's idle sleep; `Worker::run` derives `max_wait` once and uses
it for both the pop and the ceiling; the `idle_wait` doc opens with "Sleep for
`wait`" and the "Deliberately NOT" paragraph is intact.

Validation results: `cargo test --features test upload::` 75 passed / 0 failed;
`cargo fmt -p miru-agent -- --check` clean; `./scripts/lint.sh` exit 0 (only the
pre-existing, unrelated `RUSTSEC-2026-0253` allowed warning); `./scripts/covgate.sh`
upload 96.69% against the 96.00 gate; `./scripts/preflight.sh` printed
`Preflight clean`. CI on the pushed branch head: `lint`, `test`, and `tools` all
green. The PR is deliberately left in draft.

Retrospective: the plan's key call — passing the horizon as an argument rather
than storing it on `Queue` — paid off immediately in the tests, which pick a 24h
horizon independent of the 3600s production default and so can probe the boundary
without touching `UploaderOptions`. The one judgment call left to execution was
the covgate margin, and the added predicate arm and `.min()` are both exercised,
so the gate cleared with room. Everything else was mechanical.

## Context and Orientation

The upload path is an actor: the `Uploader` handle and the `Worker` run loop live in `agent/src/upload/uploader.rs`, over a `Queue` in `agent/src/upload/queue.rs`.

`QueueEntry` is `{ job, attempts, next_attempt_at }`, where `next_attempt_at: Option<DateTime<Utc>>` is an **absolute wall-clock instant** meaning "earliest instant this entry may be popped"; `None` means "eligible now". The whole queue, including these stamps, is serialized to a JSON snapshot on disk (`QueueSnapshotFile`, a `SingleThreadStateFile`) on every mutation and reloaded on startup by `Queue::from_snapshot`.

Relevant methods on `Queue`:

- `pop_ready(now)` — removes and returns the first entry whose `next_attempt_at` is `None` or `<= now`; returns `None` when nothing is eligible.
- `earliest_next_attempt()` — the minimum deadline over all entries, treating `None` as `DateTime::<Utc>::MIN_UTC`; `None` only when the queue is empty.

`Worker::run` (around line 145 of `agent/src/upload/uploader.rs`) reads `now` from the injected `now_fn`, calls `pop_ready(now)`, and on `None` with a non-empty queue computes

    let wait = match self.queue.earliest_next_attempt() {
        Some(at) => (at - now).to_std().unwrap_or(Duration::ZERO),
        None => Duration::ZERO,
    };

then calls `idle_wait(wait)`, which `select!`s the sleep against the command channel.

Backoff lives in `agent/src/cooldown/mod.rs`: `Backoff { base_secs, growth_factor, max_secs }` and `calc(backoff, exp) = min(base * growth^exp, max_secs)`. `UploaderOptions::default()` uses `base_secs: 10, growth_factor: 2, max_secs: 3600`. So **no deadline this agent stamps can ever exceed `now_at_stamp_time + max_secs`** — that invariant is the entire basis of the fix.

The bug: `.to_std().unwrap_or(Duration::ZERO)` guards only the negative direction (a deadline already in the past). Nothing bounds the positive direction. After a backward clock step, every persisted deadline is far in the future, `pop_ready` matches nothing, and `wait` is decades.

Test layout mirrors source: `agent/tests/upload/queue.rs` and `agent/tests/upload/uploader.rs`. The uploader tests have three spawn helpers at the top of `agent/tests/upload/uploader.rs`: `spawn_uploader` (real `Utc::now`, no-op sleep), `spawn_with_test_clock` (a shared clock that the recording `sleep_fn` advances by each requested duration), and `spawn_frozen` (clock frozen at spawn, sleep that never completes). None of them stamps a deadline beyond the default 3600s `max_secs`, so no uploader test changes behavior under this fix.

## Plan of Work

### `agent/src/upload/queue.rs`

**`Queue::pop_ready`** — new signature:

    pub async fn pop_ready(&mut self, now: DateTime<Utc>, max_wait: TimeDelta) -> Option<QueueEntry>

Compute the horizon once before the scan, saturating rather than panicking on a nonsense clock:

    let horizon = now
        .checked_add_signed(max_wait)
        .unwrap_or(DateTime::<Utc>::MAX_UTC);

An entry is eligible when `next_attempt_at` is `None`, or `Some(at)` with `at <= now`, or `Some(at)` with `at > horizon`. Concretely the `position` predicate becomes

    entry.next_attempt_at.is_none_or(|at| at <= now || at > horizon)

The rest of the body (`self.jobs.remove(idx)`, `persist`, `info!`) is unchanged.

Rewrite the doc comment to state the rule and why the third arm exists: `max_wait` is the largest backoff the caller's schedule can produce, so a deadline further out than `now + max_wait` cannot have been produced by that schedule and is treated as evidence of a backward clock step (unset RTC at boot, large NTP correction); such an entry is treated as due now rather than stranded forever. Keep the existing statement that `None` is returned without persisting when nothing is eligible.

Add `TimeDelta` to the existing `chrono` import line (`use chrono::{DateTime, TimeDelta, Utc};`) in the external-crates group.

**`Queue::earliest_next_attempt` doc** — delete the final sentence "The worker uses it to size an idle sleep." A low-level data structure must not document its consumer's policy. Keep the rest of the doc verbatim.

### `agent/src/upload/uploader.rs`

**`Worker::run`** — derive one horizon local at the top of the loop body, immediately after `let now = (self.now_fn)();`:

    let max_wait = TimeDelta::seconds(self.options.backoff.max_secs.max(0));

(The `.max(0)` mirrors the existing guard on `cooldown::calc(...)` in `run_attempt` against a negatively-configured backoff.) Pass it to the pop: `self.queue.pop_ready(now, max_wait).await`. In the third match arm (queue non-empty, nothing eligible), cap the computed wait with the *same* value:

    // the ceiling is defense-in-depth: with the same horizon and the same
    // `now`, pop_ready has already released any deadline beyond it, so this
    // only matters if the two ever diverge
    let ceiling = max_wait.to_std().unwrap_or(Duration::ZERO);
    let wait = match self.queue.earliest_next_attempt() {
        Some(at) => (at - now).to_std().unwrap_or(Duration::ZERO).min(ceiling),
        // unreachable: the queue is non-empty here
        None => Duration::ZERO,
    };

Deriving both from one local is the point: the eligibility horizon and the sleep ceiling cannot drift apart in a later edit.

**`Worker::idle_wait` doc** — two edits, both to the same comment block:

1. Replace the opening sentence "Wait out the shortest backoff among queued entries, staying responsive to commands." with "Sleep for `wait`, staying responsive to commands." The function never inspects the queue; it sleeps for whatever the caller computed.
2. Delete the final sentence "Any non-shutdown command — like the sleep completing — returns [`Flow::Continue`]." The sleep completing is not a command; it is the other `select!` branch.

**Keep verbatim** the middle paragraph beginning "Deliberately NOT [`Self::run_until_shutdown`]…" — that is genuine non-obvious rationale about why an enqueue must return to the run loop instead of waiting out the sleep.

### `agent/tests/upload/queue.rs`

Add a test-local helper next to `make_job`:

    /// The largest backoff a test entry could legitimately be stamped with.
    /// `pop_ready` treats any deadline beyond `now + horizon()` as a stale
    /// stamp from before a backward clock step. Comfortably larger than every
    /// deadline the other tests stamp, so only the tests that mean to probe
    /// the boundary do. A plain fn, not a `const`: `TimeDelta::hours` is not
    /// `const`.
    fn horizon() -> TimeDelta {
        TimeDelta::hours(24)
    }

Pass `horizon()` as the second argument at every existing `pop_ready` call site — the `digests` drain helper and the calls in `mod from_snapshot`, `mod requeue`, and `mod pop_ready`. Every existing assertion still holds: the largest deadline any of them stamps is `now + 1h`, well inside a 24h horizon.

Add two tests to `mod pop_ready`:

- `deadline_beyond_horizon_is_eligible_now` — requeue an entry stamped `now + 48h` (a deadline no 24h-max backoff could produce; equivalently, what a snapshot written before a backward clock step looks like). Assert `pop_ready(now, horizon())` returns it, and that its `attempts` came through unchanged. On `main` this returns `None`.
- `deadline_at_horizon_is_still_waiting` — requeue an entry stamped exactly `now + horizon()`. Assert `pop_ready(now, horizon())` is `None` and `queue.len() == 1`. This pins the clamp as `>` rather than `>=`: an ordinary maximum-backoff wait is still honored, so the fix did not turn backoff into a no-op.

No changes are needed in `agent/tests/upload/uploader.rs`: `pop_ready` is called only through the actor, and no uploader test stamps a deadline past the default 3600s `max_secs`.

Note on coverage: with both sides deriving the horizon from the same local and using the same `now`, the worker-side `.min(ceiling)` cannot change the result through the actor API — it is deliberate defense-in-depth against future divergence. It executes on every idle wait, so it is not an uncovered branch, and no test is planned for it in isolation.

## Concrete Steps

Run every command from `/home/ben/miru/workbench2/repos/agent`. This matters: `rust-toolchain.toml` pins 1.97.0, and invoking cargo from a parent directory resolves 1.94.0 and fails on the AWS SDK MSRV.

### M1 — Source fix and doc fixes

1. Edit `agent/src/upload/queue.rs` and `agent/src/upload/uploader.rs` per Plan of Work.
2. `cargo check --package miru-agent` — expect success. The integration test crate will not compile yet (`pop_ready` now takes two arguments); that is M2.
3. Confirm the doc fixes landed and the kept rationale survived:

       grep -n "idle sleep" agent/src/upload/queue.rs            # expect: no match
       grep -n "Deliberately NOT" agent/src/upload/uploader.rs   # expect: one match

4. Commit: `fix(upload): clamp retry deadlines beyond the max backoff so a backward clock step cannot stall the queue`.

### M2 — Tests

5. Edit `agent/tests/upload/queue.rs` per Plan of Work: add `horizon()`, thread it through every `pop_ready` call, add the two new tests.
6. `cargo test --features test upload::` — the `--features test` flag is mandatory; many helpers and mocks sit behind the `test` feature and omitting it fails with misleading missing-helper errors. Expect `0 failed` and both new tests present.

   Do not assert a hard total — the upload suite count moves with `main` (it was 73 as of `e07b6f5`, up from an earlier 69). Confirm the named tests explicitly:

       cargo test --features test upload::queue::pop_ready -- --list | grep horizon
       # expect: deadline_beyond_horizon_is_eligible_now, deadline_at_horizon_is_still_waiting

7. Commit: `test(upload): cover both sides of the pop_ready staleness horizon`.

### M3 — Preflight, push, CI, PR

8. `cargo fmt -p miru-agent -- --check` (never `--all`) and `./scripts/lint.sh` — both exit 0.
9. `./scripts/covgate.sh` — `agent/src/upload/.covgate` requires 96.00; expect it met. The change adds one predicate arm and one `.min()`, both exercised.
10. `./scripts/preflight.sh` — must print `Preflight clean`.
11. Push `fix/clamp-stale-retry-deadlines`; open a DRAFT PR onto `main` with the full body supplied at creation time (`gh pr create`).
12. Watch CI on the pushed branch head; take the PR out of draft only per Validation and Acceptance below.

## Validation and Acceptance

Behavioral acceptance criteria:

1. **The bug**: `pop_ready::deadline_beyond_horizon_is_eligible_now` in `agent/tests/upload/queue.rs`. One entry stamped `now + 48h`, horizon 24h → `pop_ready(now, horizon())` returns that entry. On `main` (with the one-argument signature) the equivalent call returns `None`; after the change it returns the entry.
2. **Ordinary backoff is untouched**: `pop_ready::deadline_at_horizon_is_still_waiting` — an entry stamped exactly `now + horizon()` is not popped and `len()` stays 1. The clamp is strictly `>`.
3. **No regression in existing eligibility semantics**: the pre-existing `pop_ready::skips_waiting_entries`, `pop_ready::returns_none_when_all_waiting`, and `requeue::next_attempt_at_survives_reload` all still pass unchanged apart from the new argument, including the inclusive `at <= now` boundary.
4. **Horizon is not stored on `Queue`**: the design constraint holds.

       grep -n "Backoff\|UploaderOptions\|cooldown" agent/src/upload/queue.rs   # expect: no match

5. **The two sides share one value**: in `Worker::run` in `agent/src/upload/uploader.rs`, the `max_wait` local is used both in the `pop_ready` call and to derive `ceiling`; there is no second literal or second derivation from `self.options.backoff`.
6. **Doc fixes**: `earliest_next_attempt`'s doc no longer mentions the worker's idle sleep; `idle_wait`'s doc opens with "Sleep for `wait`…" and no longer claims the sleep completing is a command; the "Deliberately NOT [`Self::run_until_shutdown`]" paragraph is intact.

Exact commands (from `/home/ben/miru/workbench2/repos/agent`) and expected results:

    cargo test --features test upload::   # 0 failed; both new tests present
    cargo fmt -p miru-agent -- --check    # exit 0, no diff
    ./scripts/lint.sh                     # exit 0
    ./scripts/covgate.sh                  # upload gate 96.00 met
    ./scripts/preflight.sh                # prints "Preflight clean", exit 0

**Gate: `./scripts/preflight.sh` must report CLEAN locally and CI must be green on the pushed branch head before the PR leaves draft.** Do not mark this work complete on a red or pending CI run.

Note: `./scripts/lint.sh` currently reports a pre-existing `RUSTSEC-2026-0253` advisory (`lru 0.16.4` via `aws-sdk-s3`) as an allowed warning; the script still exits 0. That is unrelated to this change.

## Idempotence and Recovery

Every step is an ordinary edit or a read-only check on an existing branch; all are safe to repeat. Revert uncommitted work with `git checkout -- agent/src/upload agent/tests/upload`; revert committed work with `git reset --hard <last-good-sha>`; reset the branch entirely with `git reset --hard main` (base is `e07b6f5`). `cargo check`, `cargo test`, `lint.sh`, `covgate.sh`, and `preflight.sh` have no side effects beyond `target/`. Force-push is acceptable before review starts. Nothing here touches `libs/backend-api`, `libs/device-api`, or `api/specs/` — a diff there is a mistake; revert it. No data migration is involved: existing queue snapshots on devices are read with the new rule and need no rewrite or backup.

## Out of Scope

Do not address these here; they are separate, known concerns:

- Queue capacity and eviction policy versus the ~21h retry residency of a 30-attempt entry.
- Backoff jitter.
- Unifying the `sleep_fn` / `now_fn` injection seams into a single `Clock` trait.
- The remaining upload test-coverage gaps (elapsed-time-aware idle sleep, an exact backoff assertion on the attempt-timeout path, mid-queue-pop persistence) — a separate planned PR.
