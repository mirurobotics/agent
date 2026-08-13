# Retention sink: stability-eligible delete jobs (PR 3b, first half)

## Scope

| Repo | Path | Access | Notes |
|------|------|--------|-------|
| agent | /home/ben/miru/workbench4/repos/agent | read-write | crate `miru-agent` |

Base `main` (#201 merged as `ed2b692`); branch `feat/retention-sink`. Follows the umbrella
plan's PR 3 section. The follow-up PR gives the uploader a confirm-time producer for
`require_upload: true` and removes the executor's inline ttl-0 delete.

## Purpose / Big Picture

The retention module merged with no producers or consumers. This PR gives it both ends of
the stability-eligible path (Ben's workflow, 2026-08-13):

- upload defined → send to the uploader (existing `UploadStableFileSink`, unchanged)
- retention defined and NOT `require_upload` → ALSO send to the deleter at stability

The deleter gate is `rule.retention.is_some() && !retention.require_upload` — deliberately
broader than "upload defined and not required": the same gate fires for retention-only
rules (no upload block), where `require_upload` is structurally false. `require_upload:
true` files are NOT enqueued here — their eligibility is upload confirmation, the
follow-up PR's producer.

Also wired: the deleter as a required app actor (init order deleter → uploader → scanner)
and the `workers/delete.rs` interval sweep driver — without a sweeper, sink-enqueued jobs
would accumulate in `delete_queue.json` and never execute. Driver and app wiring are
ported from the superseded #197 branch, modernized form (required actor, no enable flag).

## Progress

- [ ] M1: `retention::sink::RetentionStableFileSink` + in-source/integration tests
- [ ] M2: `workers/delete.rs` driver (port), app wiring (init_deleter, sink vec, shutdown
      ordering, ShutdownManager slot) + ported tests
- [ ] M3: Full validation, push, PR

## Surprises & Discoveries

_(filled during execution)_

## Decision Log

- **Gate covers retention-only rules** (flagged to Ben in-session): his bullets name the
  upload+retention{require_upload:false} case; the implemented gate is the superset.
- **Job's TTL clock starts at `last_observed_at`** — `retention::Job.due_at()` counts from
  `last_observed_at`, and the sink copies the StableFile's observation timestamps
  verbatim, so a stability-produced job becomes due `ttl_secs` after the observation that
  confirmed stability. No new eligibility field needed; this is why Job carries observed
  timestamps instead of a bespoke `eligible_at`.
- **Sink is concrete over `Arc<Deleter>`**, mirroring `UploadStableFileSink` over
  `Arc<Uploader>`.
- **`StableFile.retention` / upload `Job.retention` stay for now** — the executor's inline
  delete still consumes them on main; the follow-up PR removes all three together.

## Validation and Acceptance

- `./scripts/test.sh` green; `scripts/covgate.sh` all modules; import/assert linters,
  fmt, clippy, and no-features `cargo check` clean; CI green on the pushed head.
