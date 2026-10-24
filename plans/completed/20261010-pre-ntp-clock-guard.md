# Guard the upload age cap against pre-NTP clocks

Linear: ENG-1542 (File Uploads Tech Debt, M0 data-loss fixes).

## Purpose

Robots without an RTC boot with the clock at 1970 (or another stale value). The scanner stamps `first_observed_at` with that time before NTP syncs. Once NTP jumps the clock forward, the uploader's age cap (`RetryPolicy::max_age`, 7 days, which applies to every non-terminal failure since #365) read the job as decades old and dropped it on its next failure. The ledger already marked the file as reported, so it was never retried.

## Decision Log

- **Age is the retry backoff waited out, not wall-clock age.** A first-failure timestamp would itself be read from the unset clock, so instead each queue entry carries `waited_secs`, the total backoff it has waited out across all retries. The uploader passes it to `RetryPolicy::on_failure` as the job's age. It cannot be skewed by a clock jump. The field is `#[serde(default, skip_serializing_if = "is_zero")]`, so existing snapshots load and a fresh entry keeps the pinned key set.
- **Retention is out of scope.** The original version of this change also guarded the delete TTL (a 2026-10-01 sanity floor with re-stamping) and pulled back the deleter's far-future deadlines. Those parts were dropped: #361 counts delete TTLs in server time and #364 keeps retry deadlines in memory on the monotonic clock.
- **Known limit:** the tally counts only the backoff waits, not how long each failed attempt took or time the agent was stopped. A job therefore retries for longer than 7 days of wall time. This errs toward keeping the upload, so it is accepted.

## The Change

- `QueueEntry` (`agent/src/queue/snapshot.rs`) gains `waited_secs`.
- `FileQueue::retry` (`agent/src/queue/file.rs`) takes the wait and adds it to the tally.
- Uploader (`agent/src/data_uploads/upload/uploader_impl.rs`): `handle_failure` passes `waited_secs` as the age.

## Validation

- `agent/tests/data_uploads/upload/uploader.rs`: the network and counted age-cap tests use a 30s cap and step through the waits; new `failure_on_a_pre_ntp_observation_is_retried`.
- `agent/tests/data_uploads/upload/queue.rs`: `wire::waited_secs_round_trips`.
- `cargo test --package miru-agent`, `cargo fmt`, `cargo clippy --all-targets --all-features -D warnings`, custom import linter.

## What does NOT change

- The values sent to the backend (`first_observed_at`, `last_observed_at` on create).
- Retry cooldown lengths and the attempt budget.
