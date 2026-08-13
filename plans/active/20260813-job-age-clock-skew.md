# Refuse the upload age-drop when the computed job age is implausible (clock skew)

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

## Scope

| Repository | Access | Description |
|-----------|--------|-------------|
| `agent/` (this repo, root `/home/ben/miru/workbench2/repos/agent`) | read-write | Uploader retry-policy change plus tests. |

Work happens on branch `fix/job-age-clock-skew`, already created off `main` at commit `bb3952a`.

## Purpose / Big Picture

A robot with no battery-backed real-time clock (RTC) boots with an unset clock — typically 1970. The scanner stamps `first_observed_at` from that wall clock, jobs queue up while the device is offline, and then NTP steps the clock forward to the true date. On the next network-classified upload failure the uploader computes the job's age as roughly 56 years, that clears the 7-day `max_job_age` backstop, and **every queued job is dropped and erased from the durable snapshot** — the whole offline backlog is lost permanently.

After this change, an age so large it cannot be a real file age is treated as evidence of a clock correction, not as an old job: the job is requeued instead of dropped, and a distinct warning naming clock skew is logged. A genuinely old job (age past the backstop but still plausible) is still dropped exactly as before.

This is the past-side mirror of the future-side fix in PR #212 (`Queue::reset_invalid_deadlines`), which pulls back `next_attempt_at` stamps beyond any deadline the backoff schedule could have produced. Same principle: an impossible timestamp is a clock artifact, not data.

## Progress

- [ ] Milestone 1: implausible-age guard and split drop logging in `agent/src/data_uploads/upload/uploader.rs`; commit.
- [ ] Milestone 2: tests in `agent/tests/data_uploads/upload/uploader.rs`; commit.
- [ ] Milestone 3: full validation (lint, test, covgate, preflight) and push; PR leaves draft only once CI is green.

## Surprises & Discoveries

(Add entries as you go.)

## Decision Log

- Decision: Implement shape (a) — refuse the age drop when the computed age exceeds an absurd bound — via a new `UploaderOptions::max_plausible_job_age` field defaulting to `TimeDelta::days(3650)` (10 years).
  Rationale: It is the smallest change that keeps the backstop's purpose intact. The drop still fires for every age in `[max_job_age, max_plausible_job_age]`, which covers every real deployment; only ages that no device could legitimately produce are reinterpreted as skew. Putting the bound in `UploaderOptions` keeps retry policy in the uploader (the same reason PR #212 passed the horizon into `Queue` rather than storing it), and makes the boundary testable without simulating a decade. It adds no dependency, no `Job` field, and no change to the persisted snapshot format.
  Date/Author: 2026-08-13 / ben@miruml.com.

- Decision: Reject shape (b) — clamping `first_observed_at` forward to `now` when it predates a plausible floor such as the binary's build timestamp.
  Rationale: Two blocking problems. First, this repo has no build timestamp: `grep -rn "vergen\|build_timestamp\|BUILD_TIME" agent/ Cargo.toml` returns nothing and `agent/src/version/mod.rs` exposes only `CARGO_PKG_VERSION`. Producing one requires a build script or a crate like `vergen`, i.e. exactly the new dependency the constraints forbid; a hardcoded release-era date is an equally arbitrary constant with more machinery around it. Second, `first_observed_at` is not uploader-private state: it is persisted in the queue snapshot (`QueueEntry.job`), forwarded to the backend in the create-upload payload (`agent/src/data_uploads/upload/executor.rs:122`), and copied onto the retention delete job (`agent/src/data_uploads/upload/uploader.rs:231`). Clamping it would silently falsify an observability field the backend records, to fix a local retry decision. Shape (a) confines the correction to the one decision that is actually wrong.
  Date/Author: 2026-08-13 / ben@miruml.com.

## Outcomes & Retrospective

(Summarize at completion.)

## Context and Orientation

All paths below are relative to the repo root `/home/ben/miru/workbench2/repos/agent`.

**The uploader.** `agent/src/data_uploads/upload/uploader.rs` holds an actor (`Worker`) that pops jobs off a durable queue and hands each to an `UploadExecutor`. Retry policy lives in `UploaderOptions` in the same file:

- `attempts` (default 30) — total *counted* attempts before a job is dropped.
- `backoff` — growing wait between counted attempts.
- `max_job_age` (default `TimeDelta::days(7)`) — the backstop described below.

**The network-error exemption.** A failure classified as a network connection error (`UploadErr::is_network_conn_err()`) does not count against the attempt budget, because being offline is the normal condition the durable queue exists for. `Worker::handle_network_failure` handles it. Without a backstop such a job could retry forever, so the exemption is bounded by `max_job_age`, currently (around line 248):

    let age = (self.now_fn)() - entry.job.first_observed_at;
    if age >= self.options.max_job_age {
        Self::log_age_drop(&entry, age, &err);
        self.queue.remove(entry.id).await;
        return Flow::Continue;
    }

`Queue::remove` is the only point at which a job leaves durable storage — the drop is permanent.

**Where `first_observed_at` comes from.** The scanner stamps it from the wall clock when a file is first observed: `build_stable_file` in `agent/src/data_uploads/scan/rule.rs` (~line 306) copies `first_obs.timestamp` onto `StableFile`, and `agent/src/data_uploads/upload/sink.rs` copies that onto the upload `Job`. No code in this plan changes any of that.

**Clock seam.** The worker takes an injected `now_fn: Fn() -> DateTime<Utc>` and `sleep_fn`, both passed to `Uploader::spawn`. Tests drive them through helpers in `agent/tests/data_uploads/upload/uploader.rs`: `spawn_with_test_clock` (a shared `Arc<Mutex<DateTime<Utc>>>` clock that each recorded sleep advances) and, inside `mod durability`, `spawn_persisted` / `spawn_persisted_with_test_clock` plus `on_disk(&path)` for asserting what survives in the snapshot file.

**How tests build a network failure.** `MockUploadExecutor` takes scripted steps: `MockStep::Ok`, `MockStep::Err`, `MockStep::NetworkErr`, `MockStep::Hang(...)`. Existing exemption tests to imitate: `network_failures_never_drop_job`, `network_failure_uses_flat_cooldown`, `network_failure_past_max_job_age_drops_job` (~line 485). Reuse this machinery; do not add new mocks.

**Term definitions.** *Age* = `now_fn() - job.first_observed_at`. *Backstop* = the `max_job_age` drop. *Implausible age* = an age larger than any real file age this device could produce, and therefore evidence that the wall clock jumped between the stamp and now.

## Plan of Work

All production changes are in `agent/src/data_uploads/upload/uploader.rs`.

1. **Add the bound to `UploaderOptions`.** After the existing `max_job_age` field, add:

       /// Upper bound on a believable job age. An age beyond this cannot be a
       /// real file age on this device, so it is evidence that the wall clock
       /// jumped between the `first_observed_at` stamp and now (a device with
       /// no battery-backed RTC boots at 1970, then NTP steps it forward).
       /// The age backstop is not applied in that case: the job is requeued.
       pub max_plausible_job_age: TimeDelta,

   In `impl Default for UploaderOptions`, set `max_plausible_job_age: TimeDelta::days(3650)`. Also extend the `max_job_age` doc comment with a sentence pointing at the new field, so the pair reads as one policy.

2. **Guard the drop in `handle_network_failure`.** Replace the drop block so the backstop only fires on a plausible age; an implausible age falls through to the ordinary flat-cooldown requeue:

       let age = (self.now_fn)() - entry.job.first_observed_at;
       if age >= self.options.max_job_age {
           if age <= self.options.max_plausible_job_age {
               Self::log_age_drop(&entry, age, &err);
               self.queue.remove(entry.id).await;
               return Flow::Continue;
           }
           Self::log_implausible_age(&entry, age);
       }

   Boundary is deliberate: `age == max_plausible_job_age` still drops; skew is `age > max_plausible_job_age`. This mirrors PR #212's strictly-beyond horizon.

   Update the doc comment on `handle_network_failure` to state the three outcomes: drop on a plausible age past the backstop, requeue on an implausible age (suspected clock skew), requeue otherwise.

3. **Split the drop logging.** Leave `log_age_drop` (the `error!` blaming a misclassified permanent failure) untouched — it must no longer be reachable for the skew case. Add next to it:

       fn log_implausible_age(entry: &QueueEntry, age: TimeDelta) {
           warn!(
               "upload: computed job age of {} days is not a believable file age; treating it \
                as a wall-clock correction rather than an old job and keeping the job queued \
                (rule {}, file {}, digest {}, first observed at {}, attempt {})",
               age.num_days(),
               entry.job.file_rule_id,
               entry.job.file,
               entry.job.digest,
               entry.job.first_observed_at,
               entry.attempts
           );
       }

   `warn!` rather than `error!`: nothing is lost and the condition self-heals once a post-NTP scan re-stamps new files. `warn` is already imported in this file.

No other production file changes. No `Job` field, no snapshot-format change, no new dependency.

**Tests** (`agent/tests/data_uploads/upload/uploader.rs`), placed immediately after the existing `network_failure_past_max_job_age_drops_job`:

- `network_failure_with_implausible_age_keeps_job` — the regression. Default options; script `[NetworkErr, Ok]`; job with `first_observed_at = DateTime::from_timestamp(0, 0).unwrap()` (1970, simulating an unset-clock boot). Assert the executor was called twice with the same job — the job survived the network failure and was retried. Before the change this test fails: the job is dropped on the first failure and the second call never happens.
- `implausible_age_keeps_job_on_disk` — added inside `mod durability`, using `spawn_persisted_with_test_clock` and `on_disk(&path)`. Proves the job never left durable storage.
- `network_failure_past_max_job_age_drops_job` — already exists and must keep passing unchanged (8-day age, default options). It is the "backstop still works" case.
- `implausible_age_boundary_is_pinned_from_both_sides` — one test, options pinned independently of production defaults: `max_job_age: TimeDelta::days(7)`, `max_plausible_job_age: TimeDelta::days(30)`. Note `spawn_with_test_clock` seeds its clock at its own `Utc::now()`, so use ages comfortably clear of that sub-millisecond difference. Job A at exactly the bound must be **dropped**; job B beyond it must **survive** and be retried.

## Concrete Steps

Run every command from `/home/ben/miru/workbench2/repos/agent` so `rust-toolchain.toml` (1.97.0) applies. Running cargo from a parent directory resolves 1.94.0 and fails on the AWS SDK MSRV.

**Milestone 1 — production change.**

1. Edit `agent/src/data_uploads/upload/uploader.rs` per Plan of Work items 1-3.
2. Build and format:

       cargo build -p miru-agent
       cargo fmt -p miru-agent -- --check

3. Confirm the existing suite still passes:

       cargo test --features test data_uploads::upload

   Expect a line ending `0 failed`. `network_failure_past_max_job_age_drops_job` must still pass.
4. Commit:

       git add agent/src/data_uploads/upload/uploader.rs
       git commit -m "fix(upload): do not drop a job on an implausible age from a clock step"

**Milestone 2 — tests.**

5. Edit `agent/tests/data_uploads/upload/uploader.rs` per Plan of Work.
6. Verify the regression test actually captures the bug: temporarily revert the guard by hand, confirm `network_failure_with_implausible_age_keeps_job` FAILS, then restore with `git checkout HEAD -- agent/src/data_uploads/upload/uploader.rs`. If this step is skipped, the regression claim is unproven — do not skip it.
7. Run the module suite naming the new tests:

       cargo test --features test data_uploads::upload

   Expect `0 failed` and these test names present and passing:
   `network_failure_with_implausible_age_keeps_job`,
   `implausible_age_boundary_is_pinned_from_both_sides`,
   `durability::implausible_age_keeps_job_on_disk`,
   `network_failure_past_max_job_age_drops_job`.
   Do not assert a hard total for the suite.
8. Commit:

       git add agent/tests/data_uploads/upload/uploader.rs
       git commit -m "test(upload): pin the implausible-age guard and the age backstop boundary"

**Milestone 3 — validation and push.** See the next section, then:

       git push -u origin fix/job-age-clock-skew

## Validation and Acceptance

Behavioral acceptance:

1. A queued job whose `first_observed_at` is 1970 hits one network-classified failure and is **still queued and still in the snapshot file on disk**; it is retried on the next pass.
2. A job with a plausible age past `max_job_age` is **still dropped** on its first network-classified failure and removed from the queue.
3. The boundary is pinned from both sides: at `max_plausible_job_age` the job is dropped, beyond it the job survives.
4. The skew case logs a distinct `warn` naming a wall-clock correction; the existing "suspect a permanent failure misclassified as a network error" `error!` is never emitted for it.

Gates, all run from `/home/ben/miru/workbench2/repos/agent`:

    cargo fmt -p miru-agent -- --check
    ./scripts/lint.sh
    cargo test --features test data_uploads::upload
    ./scripts/test.sh
    ./scripts/covgate.sh
    ./scripts/preflight.sh

`agent/src/data_uploads/upload/.covgate` requires 96.00% for this module; the new guard branch is covered by the boundary test, so covgate must still pass.

**Preflight must report CLEAN, and CI must be green on the pushed branch head, before the PR leaves draft.** Known flake: `./scripts/preflight.sh` sometimes reports one failing component under its own parallelism, with a different component each run, while every component passes when run individually. If that happens, re-run each gate separately, confirm each is clean, and record the flake in Surprises & Discoveries — do not treat an individually-clean set of gates as a real failure, and do not treat a flaky preflight as a substitute for green CI.

## Idempotence and Recovery

Every step is repeatable. The edits are additive and confined to two files. Nothing touches on-device state, the persisted snapshot format, or any generated code under `libs/`.

Rollback: to undo a milestone, `git revert <commit>` or `git checkout HEAD -- <file>` before committing. To abandon entirely, `git checkout main` and delete the branch.

## Out of Scope

Deliberately not addressed here: queue capacity/eviction policy versus the ~21h retry residency; backoff jitter; unifying the `sleep_fn` / `now_fn` seams into a `Clock` trait; and the remaining upload test-quality gaps (the two weak `!sleeps.is_empty()` assertions at `agent/tests/data_uploads/upload/uploader.rs:351` and `:863`, and the four error-message substring assertions at `agent/tests/data_uploads/upload/queue.rs:144` and `transfer.rs:189/206/290`) — those are a separate planned PR.
