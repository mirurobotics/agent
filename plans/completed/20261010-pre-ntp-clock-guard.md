# Guard upload age and retention TTL against pre-NTP clocks

Linear: ENG-1542 (File Uploads Tech Debt, M0 data-loss fixes).

## Purpose

Robots without an RTC boot with the clock at 1970 (or another stale value). The scanner stamps `first_observed_at` and `last_observed_at` with that time before NTP syncs. Once NTP jumps the clock forward:

- **Uploads:** the next network error computed an age of decades against the 7-day backstop (`handle_network_failure` in `agent/src/data_uploads/upload/uploader.rs`) and dropped the job. The ledger already marked the file as reported, so it was never retried.
- **Retention:** a retention-only or `require_upload: false` file with a 7-day TTL had `due_at` decades in the past and was deleted at the next 60s sweep.
- **Deleter:** it never called `reset_invalid_deadlines`, so a backward jump could strand backoff deadlines far in the future.

## Progress

- [x] M1: `data_uploads::clock::floor()` sanity floor.
- [x] M2: uploader backstop counts network-failure backoff instead of wall-clock age.
- [x] M3: deleter re-stamps pre-floor observations and resets invalid deadlines every sweep.
- [x] M4: tests, lint, clippy.

## Decision Log

- **Upload backstop is clock-independent.** The issue suggests counting from the first network failure. A first-failure timestamp would itself be read from the unset clock, so instead each entry carries `network_backoff_secs`, the total backoff it has waited out after network failures. The job is dropped once that reaches `max_job_age`. With the flat 10s cooldown this tracks wall time closely and cannot be skewed by a clock jump. The field is `#[serde(default, skip_serializing_if = "is_zero")]`, so existing snapshots load and a fresh entry keeps the pinned key set.
- **Floor is a constant (2026-10-01)**, not the build date: `MIRU_AGENT_BUILD_DATE` is optional, and a compile-time constant keeps tests deterministic. Any reading before it came from an unset clock.
- **Re-stamp only once the clock is past the floor.** While the clock is still unset, stamps and "now" come from the same clock and TTLs measure correctly relative to each other. After it is set, a pre-floor stamp restarts its TTL from the first trustworthy reading, which errs toward keeping files longer.
- **Known limit:** a clock that boots at a stale but post-floor time (e.g. fake-hwclock) is not detected; that needs monotonic tracking and is out of scope.

## The Change

- New `agent/src/data_uploads/clock.rs`: `floor()` and `is_before_floor(at)`.
- `QueueEntry` gains `network_backoff_secs`; `Queue` gains `update_jobs(f)`, which persists once if any job changed.
- Uploader: `handle_network_failure` drops when `network_backoff_secs >= max_job_age`, otherwise adds the cooldown to the tally and requeues.
- Deleter `sweep`: calls `reset_invalid_deadlines(now + backoff.max_secs)` and `restamp_unset_clock_jobs(now)` before selecting entries.

## Validation

- `upload/uploader.rs`: `network_failure_past_max_job_age_drops_job` rewritten for the tally (30s budget, three 10s waits, dropped on the fourth failure); new `network_failure_on_a_pre_ntp_observation_is_retried`.
- `retention/deleter.rs` `unset_clock`: `pre_floor_observation_restarts_its_ttl`, `nothing_is_restamped_before_the_clock_is_set`, `far_future_backoff_deadline_is_pulled_back`.
- `upload/queue.rs` `wire::network_backoff_secs_round_trips`; `clock.rs` unit tests.
- `cargo test --package miru-agent`, `cargo fmt`, `cargo clippy --all-targets --all-features -D warnings`, custom import linter.

## What does NOT change

- The values sent to the backend (`first_observed_at`, `last_observed_at` on create).
- Network-failure cooldown length and attempt budget (the backoff rework is tracked separately).
