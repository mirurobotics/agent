# Upload-confirm retention producer: uploader → deleter, executor delete removed (PR 3b, second half)

## Scope

| Repo | Path | Access | Notes |
|------|------|--------|-------|
| agent | /home/ben/miru/workbench4/repos/agent | read-write | crate `miru-agent` |

Base `main` (HEAD `95bc2a5`, retention sink merged as #205); branch
`feat/upload-retention-producer`. Completes the umbrella plan's PR 3 retention workflow
(`plans/active/20260809-adopt-file-rules-spec-v0.5.md`): the stability sink covered
`require_upload: false`; this PR covers `require_upload: true` and performs the promised
3b cleanup (drop `StableFile.retention`). Supersedes the remaining producer milestone of
`plans/active/20260812-pr3a-retention-delete-worker.md` (M2 there put the producer in the
executor; Ben has since placed it in the uploader worker — see Decision Log).

## Purpose / Big Picture

A file under a rule whose retention has `require_upload: true` becomes deletion-eligible
at upload confirmation, not at stability. Today the only thing that honors that is the
executor's inline `delete_source_file`
(agent/src/data_uploads/upload/executor.rs:87-105, called at :135), which fires only on
the exact match `{require_upload: true, ttl_secs: 0}` and deletes immediately — nonzero
TTLs are silently ignored and nothing survives a restart.

This PR replaces that with a real producer (Ben's direction: "an uploader which sends
jobs to the deleter if needed once it is uploaded"):

- When the uploader worker sees an attempt succeed (upload confirmed), and the job's
  retention is `Some(r)` with `r.require_upload == true`, it enqueues a
  `retention::Job` onto the deleter. The deleter's persisted queue + sweep (merged in
  #205's world) then owns TTL scheduling and safe deletion.
- The executor's inline delete path and its tests are removed.
- The now-completable plumbing cleanup lands: `StableFile.retention` is dropped from the
  scanner's persisted ledger; the upload sink stamps `upload::Job.retention` from the
  rule it already receives.

After this PR both eligibility events produce delete jobs through the same deleter:
stability (`require_upload: false` / retention-only, via `RetentionStableFileSink`) and
upload confirmation (`require_upload: true`, via the uploader worker).

## Progress

- [x] M1: Uploader worker gains a `D: DeleterExt` handle and the confirm-time producer;
      app wiring passes the deleter into `init_uploader`
- [x] M2: Executor inline delete removed; upload sink stamps retention from the rule;
      `StableFile.retention` deleted from the ledger
- [x] M3: Tests (producer, sink stamping, dead-test removal, serde tolerance), covgate
- [x] M4: Full validation, push, PR (leave draft until CI green on the pushed head)

## Surprises & Discoveries

- No structural surprises: every file:line in the Context map was accurate at
  implementation time. The producer, wiring, and cleanup landed exactly as planned.
- Test-harness shape: rather than changing `spawn_with_test_clock`'s widely-used
  3-tuple signature, a `spawn_with_test_clock_and_deleter` variant exposes the deleter
  and the shared clock; the old helper delegates to it with a throwaway `MockDeleter`.
  The producer tests read the clock after shutdown to get the deterministic
  confirm-time `last_observed_at` (no sleeps run after a success, so the clock still
  holds the confirm instant).
- The serde-tolerance test inverted cleanly: the old `without_retention_defaults_to_none`
  removed the key from serialized JSON; its replacement `stale_retention_field_is_ignored`
  inserts a legacy `retention` object and asserts round-trip equality.
- `scripts/update-deps.sh` (house rule: run before lint) refreshed ~100 external crate
  versions in `Cargo.lock`; committed separately as a chore commit.
- Coverage after the change: `data_uploads/upload` 97.6% (gate 96.00%),
  `data_uploads/scan` 99.25% (gate 98.83%), `data_uploads/retention` 99.01% (gate
  98.39%) — all modules green.

## Decision Log

- **Producer lives in the uploader worker, not the executor** (Ben's explicit call).
  The 3a plan and the umbrella plan (lines 132-133) had `LiveExecutor` enqueue on
  confirm; the worker is the better seam: it already owns the success/failure decision
  (`AttemptOutcome::Succeeded`, agent/src/data_uploads/upload/uploader.rs:197-201 —
  since #204 a job stays queued until confirm, so `Succeeded` *is* "confirmed") and it
  has an injectable clock (`now_fn`, from #186; uploader.rs:144,155). The executor stays
  a pure transfer mechanism; retries/timeouts never see retention.
- **TTL clock counts from the confirm instant.** `retention::Job.due_at() =
  last_observed_at + ttl_secs` (agent/src/data_uploads/retention/job.rs:24-30). A
  require_upload file becomes eligible at upload confirmation (v0.5 spec), and uploads
  can retry for hours — counting from the scan-time observation would delete
  late-confirmed files instantly. So the producer stamps:
  - `first_observed_at` — copied from the upload job (provenance, and the field the
    uploader's own age-drop logic keys on);
  - `last_observed_at = (self.now_fn)()` at the confirm instant, so `due_at` counts
    from eligibility.
  Field-name tension, acknowledged: the field reads "last observed" but here it holds
  the confirm time — the moment the uploader last dealt with the file. `retention::Job`
  was deliberately kept event-agnostic ("jobs name *when* a file became deletable,
  never *why*" — 3a plan Decision Log); the eligibility instant riding in
  `last_observed_at` is that design's cost. Renaming the field (e.g. `eligible_at`)
  would touch the stability sink and the persisted `delete_queue.json` shape and is out
  of scope; note it as a candidate follow-up if the ambiguity bites.
- **Gate**: `job.retention` is `Some(r) && r.require_upload` → enqueue. With
  `require_upload: false` the uploader does NOT enqueue — those files were already
  enqueued at stability by `RetentionStableFileSink`
  (agent/src/data_uploads/retention/sink.rs:40-50 skips the require_upload case for the
  mirror-image reason); enqueueing here too would double-enqueue. Gate stays at the
  producer (not "have the sink stamp retention only when require_upload") so
  `Job.retention` remains the rule's full retention block and policy reads in one place.
- **Enqueue failure is swallowed** (warn-logged): the upload is already durably
  confirmed; a full delete queue or dead deleter must never turn a confirmed upload
  into a failed attempt (which would re-drive the upload). Mirrors the sinks'
  infallible-enqueue posture (upload/sink.rs:56-58, retention/sink.rs:64-66).
- **`upload::Job.retention` stays** (agent/src/data_uploads/upload/job.rs:19-20). The
  producer reads it at success time and the uploader has no access to the rule (rules
  live in the scanner's config world; the queue persists jobs across restarts and
  rule-set changes). It is the retention policy's carrier from sink to producer.
- **`StableFile.retention` dies** (agent/src/data_uploads/scan/state.rs:119-121). It
  exists only to ferry the rule's retention from the scanner into the upload sink's Job
  (stamped in `build_stable_file`, agent/src/data_uploads/scan/rule.rs:287-313; copied
  at agent/src/data_uploads/upload/sink.rs:53). Post-#198 sinks receive the full
  `FileRule`, so the upload sink stamps `job.retention = rule.retention.clone()`
  directly and the ledger field is vestigial — the umbrella plan's promised 3b cleanup
  ("drop `StableFile.retention`; the ledger keeps only scan facts").
- **Serde tolerance of the ledger field removal**: `StableFile` derives plain
  `Deserialize` with no `deny_unknown_fields` (state.rs:108-109; repo-wide grep for
  `deny_unknown_fields` is empty), so old `scanner.json` snapshots carrying a
  `retention` key deserialize fine — serde ignores unknown fields by default. No
  snapshot invalidation, no migration.
- **Timing/priority of enqueue vs queue removal**: enqueue the delete job *before*
  `self.queue.remove(entry.id)` is not load-bearing either way — a crash between the
  two re-drives the upload (queue entry still on disk), and the re-confirmed upload
  re-enqueues a delete; duplicate delete jobs are harmless (the sweep re-stats and the
  second finds the file `AlreadyGone`). Implement in whichever order reads cleanest in
  `run_attempt`; record the crash-window reasoning in a comment.

## Context and Orientation

- **Success point**: `Worker::run_attempt`, `AttemptOutcome::Succeeded` arm
  (uploader.rs:197-201) — bumps attempts, logs, `queue.remove`. The producer hook goes
  here. `Worker` fields (uploader.rs:131-144): `receiver, queue, executor, options,
  sleep_fn, now_fn` — add `deleter: Arc<D>` with a new `D: DeleterExt + 'static` type
  parameter (mirroring how the worker is already generic over `ExecutorT:
  UploadExecutor`). `DeleterExt` methods return `impl Future + Send`
  (agent/src/data_uploads/retention/deleter.rs:203-217), so the worker's spawned future
  stays provably `Send`.
- **`Uploader::spawn`** (uploader.rs:416-445) gains the `deleter: Arc<D>` parameter.
  Call sites to update:
  - agent/src/app/state.rs:229 (`init_uploader`) — the deleter is already constructed
    first (state.rs:96-100, init order deleter → uploader → scanner, comment at
    :96-97), so `init_uploader` just grows an `Arc<retention::Deleter>` argument.
  - agent/tests/data_uploads/upload/uploader.rs:68, :97, :109, :542, :751 (the
    `spawn_uploader` / `spawn_with_test_clock` / `spawn_frozen` helpers and two inline
    spawns).
  - agent/tests/data_uploads/upload/sink.rs:75 (upload-sink harness).
  Test call sites pass `MockDeleter::new()` (agent/tests/mocks/deleter.rs — records
  every enqueued `retention::Job`, scriptable `Err` steps).
- **New import edge**: `upload::uploader` imports
  `crate::data_uploads::retention::{DeleterExt, Job}` (alias, e.g. `Job as DeleteJob`,
  to avoid clashing with `upload::job::Job`). retention imports nothing from upload
  (retention/sink.rs imports scan only), so no cycle. `.lint-imports.toml` enforces
  only grouping/labels, not module edges.
- **retention::Job shape** (retention/job.rs:8-19): `file, size, digest, mtime,
  first_observed_at, last_observed_at, ttl_secs, file_rule_id, deployment_id` — every
  field except the timestamps/ttl copies straight off `upload::Job`
  (upload/job.rs:9-21). The sweep re-stats size+mtime and re-hashes on mtime drift
  before deleting (deleter.rs:114-120 and the 3a plan's sweep-safety entry), so a file
  rewritten after confirm is never deleted by a stale job.
- **Executor delete path to remove**: `LiveExecutor::delete_source_file`
  (executor.rs:87-105) + its call (executor.rs:135) + now-unused imports
  (`crate::filesys::files`, `crate::models::FileRuleRetention`, `warn`). Tests that die
  with it: the `// retention` block in agent/tests/data_uploads/upload/executor.rs:389-483
  (`retention_setup`, `require_upload_retention_deletes_source_after_confirm`,
  `no_retention_leaves_source_in_place`, `delete_failure_after_confirm_still_succeeds`,
  `missing_source_at_delete_is_success`) plus their now-unused imports.
- **`StableFile.retention` removal sites** (field + `retention: ...` in constructors):
  - agent/src/data_uploads/scan/state.rs:119-121 (field), :215 (test helper), and the
    `without_retention_defaults_to_none` serde test at :696-706 (dies with the field).
  - agent/src/data_uploads/scan/rule.rs:287-313 (`build_stable_file` loses its
    `retention` param; `differs_from_previous` stops cloning `state.rule().retention`),
    test helpers at :444, :470, and the `stamps_retention_from_rule` /
    `stamps_no_retention_from_default_rule` tests at :895-923 (die — stamping moves to
    the upload sink, which is where the replacement test lives).
  - agent/src/data_uploads/scan/scanner.rs:1023, :1637 (test constructions).
  - agent/tests/data_uploads/retention/sink.rs:22, agent/tests/data_uploads/upload/sink.rs:39
    (test constructions).
- **Upload sink change** (upload/sink.rs:44-54): `retention: file.retention` →
  `retention: rule.retention.clone()`. Test agent/tests/data_uploads/upload/sink.rs:137
  currently expects `stable.retention`; it asserts the rule's retention instead.

## Plan of Work

- **M1 — producer.** Thread `Arc<D: DeleterExt>` through `Worker` and
  `Uploader::spawn`; in the `Succeeded` arm build the `retention::Job`
  (`first_observed_at` from the upload job, `last_observed_at = (self.now_fn)()`,
  `ttl_secs` from `job.retention`) gated on `retention.require_upload`, enqueue,
  warn-and-swallow on error. Update `init_uploader` (app/state.rs) to take and pass the
  already-constructed deleter handle.
- **M2 — cleanup.** Remove `delete_source_file` and its call; upload sink stamps
  retention from the rule; delete `StableFile.retention` and every construction-site
  `retention:` line listed above; drop dead imports.
- **M3 — tests.**
  - Uploader producer tests (agent/tests/data_uploads/upload/uploader.rs, harness:
    `spawn_with_test_clock` clock + `MockDeleter`):
    - success with `require_upload: true, ttl_secs: N` → deleter records exactly one
      whole `retention::Job` (assert the full struct, per the struct-assertion
      convention): fields copied from the upload job, `ttl_secs = N`,
      `last_observed_at` = the test clock's value at confirm, `first_observed_at` =
      the upload job's.
    - success with `require_upload: false` → nothing enqueued (double-enqueue guard).
    - success with `retention: None` → nothing enqueued.
    - deleter enqueue `Err` (scripted `MockStep::Err`) → upload still confirmed/removed
      from the queue, worker continues (e.g. a subsequent job still runs).
    - failed / retried / terminal outcomes → nothing enqueued.
  - Upload sink test: job's retention comes from the rule (replaces the scan-side
    `stamps_retention_from_rule` pair); retention-only-rule and expected-job tests
    updated for the field moves.
  - Scanner ledger serde: keep a tolerance test proving an old snapshot containing a
    `retention` key still deserializes (inverse of the dying
    `without_retention_defaults_to_none` — now asserts the unknown key is ignored).
  - Remove the executor retention test block.
  - `scripts/covgate.sh` all modules (upload gains logic — expect covgate to hold or
    rise; scan loses only a stamped field).
- **M4 — validation + PR.** Below; PR references the umbrella plan and closes out the
  PR 3 section (retention workflow complete: stability + upload-confirm producers).

## Validation and Acceptance

- `./scripts/test.sh` green; `scripts/covgate.sh` all modules pass.
- Import linter + field-by-field-assert linter, `cargo fmt` check, clippy `-D warnings`,
  and no-features `cargo check` all clean (`scripts/lint.sh` after
  `scripts/update-deps.sh`).
- Preflight reports CLEAN — CI green on the pushed branch head — before the PR leaves
  draft.
- Behavior: a confirmed upload under `require_upload: true` yields a queued delete job
  due `ttl_secs` after the confirm instant (deleted by the sweep thereafter);
  `require_upload: false` and retention-absent uploads enqueue nothing at confirm; no
  code path deletes a source file inline at confirm anymore.
