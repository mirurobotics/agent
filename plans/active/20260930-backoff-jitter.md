# Backoff jitter: stop fleet-wide retries from firing in lockstep

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

**Status: parked, not started.** This file exists so the finding is tracked. Nobody has committed to it. Review finding 5 from the review of #186's follow-up work was set aside on purpose. Pick it up only when there is a decision to do it.

## Scope

| Repository | Access | Description |
| --- | --- | --- |
| `agent` | read/write | Rust workspace for the Miru device agent. All edits, validation, and commits happen here. |

## Purpose / Big Picture

`cooldown::calc` (`agent/src/cooldown/mod.rs`) is fully deterministic: `min(base * growth^exp, max)`. Devices that fail together also retry together, forever. The upload worker is where the finding surfaced. A `401` is non-terminal, so a fleet-wide auth outage is exactly the failure that retries hardest and most synchronously. When the backend recovers, the whole fleet hits it on the same second.

After this change, retry delays carry bounded randomness so a fleet that failed together spreads its retries out. No single device's worst-case wait gets longer.

## Context the handoff got wrong

The original note estimated "~3 lines: `next_attempt_at = now + wait * rand(0.5..1.5)`". That is not safe as written, for four reasons.

1. **Jitter above `max_secs` is silently clamped, and the clamping re-synchronizes the fleet.** The upload run loop calls `Queue::reset_invalid_deadlines(now + max_secs)` (#212, now in `agent/src/data_uploads/queue/mod.rs`) before every idle wait. Any deadline past that horizon is pulled back to exactly `now + max_secs`, persisted with a full snapshot rewrite, and logged with a `warn!`. With `rand(0.5..1.5)`, about half of all retries at the 3600s cap land past the horizon and snap back to one value. That undoes the jitter exactly where a long outage spends its time, and adds spurious warnings and disk writes. **Jitter must never push a wait above `max_secs`.**
2. **There is no RNG dependency.** Neither `Cargo.toml` depends on `rand`. Options:
   - add a small crate (`fastrand`)
   - derive a stable per-job offset by hashing the job digest, which needs no dependency and is reproducible across restarts
3. **Tests pin exact durations on purpose.** #214 made backoff assertions exact:
   - `retry_backoff_follows_expected_sequence` and `network_failure_uses_flat_cooldown` in `agent/tests/data_uploads/upload/uploader.rs`
   - `the_delay_grows_and_caps` in `retention/deleter.rs`

   Jitter needs a seam, such as a jitter fraction in `UploaderOptions` that tests set to 0, or a jitter source they can inject. It must not weaken those assertions.
4. **`cooldown::calc` has seven callers, not one:** `deploy/fsm.rs`, `app/upgrade.rs`, `retention/deleter.rs`, `upload/uploader.rs`, `sync/syncer.rs`, `workers/token_refresh.rs`, `workers/mqtt.rs`. MQTT reconnect after a broker outage and the syncer's poll backoff are arguably worse lockstep offenders than uploads. Deciding the scope is the first decision to make.

## Open decisions

- **Scope.** Options:
  - uploader only, which is what the finding covers
  - every `cooldown::calc` caller, via a jittered variant in `cooldown`
  - uploader, MQTT and syncer, the fleet-synchronous retry loops
- **Shape.** Candidates:
  - "equal jitter": `wait/2 + rand(0..=wait/2)`
  - "full jitter": `rand(0..=wait)`

  Both stay at or below the deterministic value, so they cannot cross the #212 horizon. Full jitter spreads the load best but can retry almost immediately.
- **Network-classified failures.** `handle_network_failure` uses a flat `base_secs` cooldown, not `cooldown::calc`. A fleet-wide network outage recovers in lockstep too, so decide whether the flat cooldown also gets jitter.
- **Randomness source.** A new crate, or a hash of the job digest (stable per job, so a restart does not re-roll it).

## Plan of work (once decided)

1. Add the jittered computation, bounded so the result is at most the deterministic `calc` value. Put it in `cooldown` if the scope is several callers.
2. Thread a jitter control through `UploaderOptions` (and other callers' options if in scope), defaulting to on in production and off in tests that pin exact durations.
3. Add tests:
   - jittered waits stay within `[lower bound, deterministic value]`
   - at the cap, jittered deadlines never trip `reset_invalid_deadlines`; assert no `warn!` and no extra persist
   - two identically-failing jobs get different deadlines
4. Verify by perturbation: remove the upper bound and confirm the cap test fails.

## Progress

- [ ] Scope and shape decided
- [ ] Implementation
- [ ] Tests plus perturbation check

## Surprises & Discoveries

- 2026-09-30: the #212 horizon interaction (item 1 above) was found while writing this plan. It rules out any jitter factor above 1.0.

## Decision Log

- 2026-08: the finding was set aside by request ("ignore this for now").
- 2026-09-30: this plan opened as a draft PR so the finding stays tracked. No code change.

## Outcomes & Retrospective

Not started.
