# PR 3a — Retention delete worker: pending-delete queue + upload-confirm producer

## Scope

| Repo | Path | Access | Notes |
|------|------|--------|-------|
| agent | /home/ben/miru/workbench4/repos/agent | read-write | All changes in crate `miru-agent` |

Third PR of the umbrella plan `plans/active/20260809-adopt-file-rules-spec-v0.5.md`
(PR 3 section, revised 2026-08-12). Base `main` (PR 2 merged as `fc7333c`); working
branch `feat/retention-delete-worker`. Supersedes draft PR #191 — its queue/actor/sweep
code is ported here onto the `FileRule` world; #191 closes when this opens.

## Purpose / Big Picture

Deletion of uploaded files is currently an inline step in the upload executor
(`LiveExecutor::upload` → `delete_source_file`, exact-match on
`retention == Some({require_upload: true, ttl_secs: 0})`). That shape cannot express
v0.5 retention: nonzero TTLs need a clock that survives restarts, and retention-only
rules (PR 3b) have no upload to hang deletion off.

This PR makes deletion a first-class subsystem, deliberately **outside the scanner**
(Ben's call: the scanner shouldn't know about retention — it emits stable files with
their full rule attached, and downstream subsystems own their own policy):

- `agent/src/delete/`: persisted pending-delete queue (`delete_queue.json`) of
  `PendingDelete` records + `Deleter` actor with a metadata-and-digest-checked sweep.
- `LiveExecutor` enqueues a pending delete on confirmed upload when
  `retention.require_upload` is true (any `ttl_secs`), instead of deleting inline.
  Enqueue failure never fails the already-confirmed upload.
- `workers/delete.rs`: interval driver, default 60s, mirroring `workers/scan.rs`.
- App wiring with fail-open init: a deleter spawn failure degrades to
  uploads-without-deletion, never a boot failure.

Not in this PR (→ 3b): the stability-eligible producer for retention-only /
`require_upload: false` rules, and dropping `StableFile.retention`.

## Progress

- [ ] M1: Port `delete/` module (queue, deleter actor, errors) from #191, adapted to FileRuleRetention
- [ ] M2: Executor enqueues on confirm; inline delete removed
- [ ] M3: Interval worker + app wiring (options, state, shutdown ordering)
- [ ] M4: Tests (port + adapt #191's), covgate, preflight, push

## Surprises & Discoveries

_(filled as work proceeds)_

## Decision Log

- **Standalone delete subsystem, scanner retention-unaware** (Ben, 2026-08-12) — see the
  umbrella plan's revised PR 3 section for the full rationale and rejected alternatives
  (scan-tick sweep with ledger eligibility; direct scanner handle for confirms).
- **`PendingDelete` keeps #191's event-agnostic shape** — records name *when* a file
  became deletable, never *why*; 3b adds a producer, not fields. Changes from #191:
  `delete_delay_secs: i64` → `ttl_secs: u64` (from `FileRuleRetention`, so no negative
  clamp), `upload_rule_id` → `file_rule_id`.
- **Enqueue condition**: `job.retention` is `Some` with `require_upload: true` — the
  `ttl_secs: 0` exact-match at the delete site is gone; nonzero TTLs now work. With
  `require_upload: false`, the executor does NOT enqueue (that file's eligibility is
  stability, the 3b producer's job — enqueueing here too would double-enqueue).
- **Timing change, accepted**: today's inline delete-at-confirm becomes
  delete-at-next-sweep (≤ ~60s later) for ttl-0 rules. Zero production users.
- **Sweep safety (ported from #191)**: only paths carried by `PendingDelete` records are
  ever deleted; each due entry is re-stat'd — size+mtime match → delete; mtime-only
  change → re-hash, delete only on digest match; any other mismatch or a vanished file →
  drop the record without deleting.

## Context and Orientation

- Delete site today: `agent/src/upload/executor.rs:82` (`delete_source_file`), called at
  `:130` after confirm.
- `Job.retention: Option<FileRuleRetention>` already carries `{require_upload, ttl_secs}`
  (threaded in PR 1).
- #191 reference code (branch `origin/feat/delete-worker`): `agent/src/delete/{queue,deleter,errors,mod}.rs`,
  `agent/src/workers/delete.rs`, app wiring in `app/{options,run,state}.rs`, disk layout
  `delete_queue.json`, mocks `agent/tests/mocks/deleter.rs`, tests
  `agent/tests/{delete/,workers/delete.rs,app/state.rs}`. Its `models/upload_rule.rs` /
  scan-side changes are obsolete (pre-PR-1/2 world) and are not ported.
- Persistence pattern: `SingleThreadStateFile` snapshot, mirroring the uploader's queue.

## Plan of Work

- **M1** — Port `delete/` with vocabulary updates; covgate file for the new module.
- **M2** — `LiveExecutor` gains a `DeleterExt` handle (as it holds transfer/token_mngr);
  `delete_source_file` → `enqueue_pending_delete`; eligible_at = confirm time.
- **M3** — `workers/delete.rs` driver; `AppOptions`/`AppState` spawn deleter alongside
  uploader; shutdown ordering covered by tests.
- **M4** — Port #191's test surface (deleter sweep cases, worker driver, executor
  enqueue-on-confirm, app wiring/shutdown), adapt to `FileRuleRetention`; new cases:
  nonzero TTL due-at, `require_upload: false` does not enqueue.

## Validation and Acceptance

- `./scripts/test.sh` green; `scripts/covgate.sh` all modules (new `delete/.covgate`).
- `scripts/lint.sh` clean; `cargo check` with no features clean.
- Behavior: confirmed upload under `require_upload: true, ttl_secs: 0` still results in
  the source file's deletion (now within one sweep interval); `require_upload: false`
  and retention-absent rules delete nothing.
