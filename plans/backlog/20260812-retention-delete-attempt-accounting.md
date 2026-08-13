# Retention delete worker — attempt accounting (give up after 10 attempts)

## Scope

| Repo | Path | Access | Notes |
|------|------|--------|-------|
| agent | /home/ben/miru/workbench1/repos/agent-wt-delete-attempts | read-write | All changes in crate `miru-agent`, module `agent/src/data_uploads/retention/` (plus its integration tests under `agent/tests/data_uploads/retention/`) |

Branch `feat/retention-delete-attempts`, rebased onto `main` at `6d1a9b3`. The branch's own
commits are `4b8a2de` (this plan) and `c32f306` (its refinement); `c32f306` is the branch
head at which implementation starts. No other repo is read or written. No API spec, no
infra, no backend change.

## Purpose / Big Picture

The retention delete worker (`agent/src/data_uploads/retention/`) keeps a persisted queue
of *delete jobs*: records saying "file X, whose size/mtime/digest were Y, may be deleted
after time T". A background driver sweeps the queue every 60 seconds; each due job is
re-stat'd, its identity re-checked, and the file unlinked.

Today a job whose sweep fails — a stat error, a hash error, an unlink error — is requeued
at the tail with no memory that it failed. It will be retried every 60 seconds **forever**,
logging a `warn!` each time. A file sitting on a read-only mount, or a path that has become
a directory, produces one warning per minute for the life of the process and never
resolves.

After this change the queue remembers how many times each job has been attempted. A job
whose failure class can never succeed is dropped on the first failure; any other failure
consumes one attempt, and at 10 consumed attempts (≈10 minutes at the default sweep
interval) the job is dropped from the queue with a single loud `error!` naming the file,
rule, deployment, digest and attempt count. The attempt count is persisted, so it survives
an agent restart.

Observable after this change: with a wedged delete target, `journalctl`/agent logs show at
most 10 `warn!` lines followed by exactly one `error!` line, instead of an unbounded warn
stream; the persisted queue at `<agent root>/delete_queue.json` shrinks to `{"entries":[]}`
instead of holding the wedged entry forever.

**The dropped job is not retried and the file is not deleted** — a deliberate,
fail-open trade; the reasoning and the evidence that no resurrection loop exists are in
D7 and in "How delete jobs are minted" below.

## Progress

- [ ] M1: `attempts` on the existing `QueueEntry`; `DEFAULT_ATTEMPTS` + `DeleterArgs.attempts`; split `SweepOutcome::Retry` into `CountedRetry`/`TerminalFailure` classified by `io::ErrorKind`; drop on exhaustion/terminal with `error!`; fix the tests whose meaning or shape changes
- [ ] M2: New tests (classifier, sweep, restart, integration); covgate re-ratchet
- [ ] M3: `./scripts/preflight.sh` clean, push, CI green, PR out of draft

## Surprises & Discoveries

_(filled as work proceeds)_

## Decision Log

- **D0 — rebase onto `6d1a9b3`; two upstream PRs moved the ground under M1**
  (2026-08-12). `2777810` (*refactor(retention): make the delete queue's durability
  structural*, #206) and `08b57c5` (*feat(upload): enqueue retention jobs at upload
  confirmation*, #207) landed after this plan was first written. #206 introduced
  `QueueEntry { id, job }` and a `Vec<QueueEntry>` snapshot, replaced
  `pop_front`/`requeue(Job)` with `next_ready`/`remove(id)`/`requeue(QueueEntry)`, made
  every mutation persist as its last act, and moved `queue.rs`'s unit tests out to
  `agent/tests/data_uploads/retention/queue.rs`. #207 added a second producer of delete
  jobs. Consequences: **D1** shrinks to one field on an entry that already exists, **D2**
  inverts (the change is now backward compatible), and **D6** is satisfied structurally
  rather than by a new call. **D3, D4, D5, D7 are unchanged** — the cap, the classification,
  the no-backoff decision and the loud-drop decision are all independent of how the queue
  stores or persists entries.

- **D1 — `attempts` goes on the existing `retention::queue::QueueEntry`**
  (2026-08-12, revised after D0). Add
  `#[serde(default)] pub attempts: u32` to `QueueEntry`
  (`agent/src/data_uploads/retention/queue.rs:19-27`). Do **not** introduce a second entry
  type: #206 already established `QueueEntry { id, job }` as the queue's own record about a
  job, which is exactly where bookkeeping belongs. `Job`
  (`agent/src/data_uploads/retention/job.rs`) stays the pure domain record minted by the
  producers and compared whole in tests. The earlier version of this plan argued *against*
  an `id: Uuid` field; that question is moot — the id exists, and it is load-bearing:
  `Queue::remove(id)` and `Queue::requeue(entry)` address entries by it, and same-path
  duplicate jobs are explicitly allowed (`duplicate_same_path_jobs_are_both_queued`,
  `same_path_duplicate_resolves_in_one_sweep`), so no field of `Job` is a usable key. The
  shape now matches the uploader's `QueueEntry { id, job, attempts, next_attempt_at }`
  (`agent/src/data_uploads/upload/queue.rs`) minus the backoff stamp — see D5.

- **D2 — adding `attempts` is backward compatible; nothing resets** (2026-08-12, revised
  after D0). The migration cost was already paid by #206, whose own doc comment on
  `QueueEntry::id` records that "the entry shape itself changed from a flat `Job` to
  `{id, job}`". Because the snapshot is *already* `{"entries":[{"id":..,"job":{..}}, ..]}`,
  `#[serde(default)]` on `attempts` makes a snapshot written by #206 load cleanly with
  `attempts: 0`. No on-disk state is lost by this change. A *pre-#206* flat-`Job` snapshot
  still fails to parse, and `SingleThreadStateFile::new_with_default`
  (`agent/src/filesys/state_file.rs:54-63`) falls back to writing the empty default — but
  that reset is #206's, not this change's, and it is fail-open either way (files are
  *retained*, never wrongly deleted).

- **D3 — the limit is a documented option with a named default, not a bare literal**
  (2026-08-12). `DeleterArgs` (`agent/src/data_uploads/retention/deleter.rs:33-47`) is the
  only knob struct the deleter has — there is no `DeleterOptions` — so the field goes
  there: `pub attempts: u32`, documented "Total sweep attempts per job before it is
  dropped.", defaulted from a new `const DEFAULT_ATTEMPTS: u32 = 10;` next to the existing
  `const DEFAULT_QUEUE_CAPACITY: usize = 4096;` (`deleter.rs:21`). This mirrors
  `UploaderOptions.attempts: u32` (`upload/uploader.rs:42`, default `30` at
  `uploader.rs:65`).

- **D4 — classification is a split of `SweepOutcome`, driven by `io::ErrorKind`**
  (2026-08-12). `SweepOutcome::Retry` (`deleter.rs:54-64`) becomes two variants,
  `CountedRetry` and `TerminalFailure`, decided at the three producing sites.
  `FileSysErr` already exposes enough to discriminate — every relevant variant carries a
  public `source: Box<std::io::Error>` — so **no new predicate on `FileSysErr` is added**
  (the task allowed adding one; it is unnecessary). A private helper in `deleter.rs`
  extracts the kind. Terminal kinds: `PermissionDenied`, `ReadOnlyFilesystem`,
  `IsADirectory`, `NotADirectory`, `InvalidFilename`. Everything else — including any
  `FileSysErr` variant with no extractable `io::Error` — is counted, on the principle that
  the attempt cap is precisely the backstop for failures we cannot classify. A vanished
  file is *not* a failure at all: `files::metadata` maps `NotFound` to
  `FileSysErr::PathDoesNotExistErr` and `stat_file` (`deleter.rs:120-135`) already returns
  `SweepOutcome::AlreadyGone` for it. This is the lesson of the upload side's
  `is_terminal`/`is_network_conn_err` bools (`upload/errors.rs`): state deliberately
  which classes consume budget, and log the rest loudly.

- **D5 — no per-job exponential backoff** (2026-08-12). Deliberate divergence from upload.
  `agent/src/workers/delete.rs` already paces the retries at a fixed
  `Options.sweep_interval_secs` (default 60), so 10 attempts is ≈10 minutes of retrying and
  a `next_attempt_at` field would only add state without changing the cadence. Upload needs
  backoff because its executor is driven by network work, not by a fixed sweep. Adding
  `cooldown::Backoff` (`agent/src/cooldown/mod.rs`) is a clean follow-up if 10 minutes
  proves too short.

- **D6 — the counted-retry requeue must persist; #206 made that structural**
  (2026-08-12, revised after D0). The requirement stands and is *why* the counter is
  durable: an unpersisted increment would reset on every restart and the cap would never
  fire. But the implementation is now free. `Queue::enqueue`, `Queue::remove` and
  `Queue::requeue` each persist as their last act
  (`queue.rs:44-47`, `queue.rs:104`, `queue.rs:130`, `queue.rs:148`), so
  `self.queue.requeue(entry).await` on the counted-retry path already writes the
  incremented counter to disk. **No new `persist()` call is added, and `Queue::persist` is
  not made public.** Write volume is likewise unchanged from today: one snapshot write per
  requeued job per sweep, bounded by the 60s sweep interval and by the fact that a failing
  job is now dropped after 10 sweeps rather than requeued forever. `attempts_survive_a_restart`
  (M2) pins the durability behaviorally so a future refactor of `Queue` cannot silently
  break it.

- **D7 — exhaustion drops the job and leaks the file, loudly** (2026-08-12). On the 10th
  counted failure, or on the first terminal failure, the entry is removed via
  `Queue::remove(entry.id)` — which persists — and an `error!` is emitted naming file path,
  `file_rule_id`, `deployment_id`, `digest` and attempt count, mirroring `log_dropped` /
  `log_terminal_drop` in `upload/uploader.rs:363-368` and `:383`. Nothing extra is persisted
  to stop the job coming back, because nothing brings it back — see "How delete jobs are
  minted" for the per-producer argument. So the job is gone for good and the file stays on
  disk, forgotten — fail-open, never fail-deadly. Corollary, and this is correct behavior
  rather than a loop: if the file is later **modified**, the scanner legitimately re-emits
  it as a new stable file, a producer mints a fresh `Job`, and it gets a fresh budget of 10.

- **D8 — three milestones, not four: the data-shape change cannot stand alone**
  (2026-08-12, added with D0). The earlier split was M1 = data shape, M2 = policy. After
  D1 the data-shape half is one struct field, one construction site, one const, one
  `DeleterArgs` field and one test accessor — and it *cannot compile clean on its own*: CI
  clippy runs with `-D warnings`, and the new private `SingleThreadDeleter.attempts` field
  and the new `#[cfg(test)] queue_entries()` accessor have no reader until the policy lands,
  so a shape-only commit is a `dead_code` failure. Shape and policy therefore land as one
  reviewable commit (M1), tests as the next (M2), validation as the last (M3).

## Context and Orientation

Read this section as if you have never opened this repository. Every path below is
relative to the repo root `/home/ben/miru/workbench1/repos/agent-wt-delete-attempts`.

### Terms

- **Delete job** — `agent/src/data_uploads/retention/job.rs` (68 lines), `pub struct Job`:
  an all-public record of the file to delete, its recorded identity (size, digest, mtime),
  its observation window, its TTL and its owning rule/deployment. Nine fields; the literal
  `Job { .. }` construction is spelled out in M2. Derives
  `Clone, Debug, PartialEq, Serialize, Deserialize`. `Job::due_at()` returns
  `last_observed_at + ttl_secs`, saturating to `DateTime::<Utc>::MAX_UTC` ("never due") on
  overflow. There is no id and no attempt count — both live one level up, on `QueueEntry`.
- **Queue entry** — `retention::queue::QueueEntry { id: Uuid, job: Job }`
  (`queue.rs:19-27`). The queue's own record about a job. `id` is minted at enqueue
  (`queue.rs:100-103`); two identical jobs for one path legitimately coexist, so no field
  of `Job` is a usable key. This plan adds `attempts` here.
- **Sweep** — one pass of `SingleThreadDeleter::sweep` (`deleter.rs:86-104`). Budget:
  `queue.count_ready(now)`, one visit per entry due at `now`. Each iteration selects
  `queue.next_ready(now)` — a **clone**; selection never mutates and never persists —
  resolves it, then either `requeue`s it at the tail or `remove`s it by id. Requeueing at
  the tail puts the entry behind every not-yet-visited due entry, so the budget is exactly
  enough to visit each due entry once and never twice; if drops empty the due set early,
  `next_ready` returns `None` and the `else { break }` ends the pass.
- **Sweep driver** — `agent/src/workers/delete.rs`. `Options { sweep_interval_secs: i64 }`,
  default 60. Runs one sweep immediately at boot, then sleeps and sweeps forever. It is the
  only thing that imposes a cadence; the deleter actor is purely reactive.
- **Snapshot** — `delete_queue.json`, path from `agent/src/disk/layout.rs:53-55`
  (`delete_queue()` = `root()/delete_queue.json`). Written through
  `SingleThreadStateFile` (`agent/src/filesys/state_file.rs`), which caches state in memory
  and writes atomically. `new_with_default` (`state_file.rs:54-63`) writes the default on
  *any* read or parse failure — see D2.
- **Covgate** — a per-module `.covgate` file holding a minimum line-coverage percentage,
  enforced by `./scripts/covgate.sh`. Current values:
  `agent/src/data_uploads/retention/.covgate` = `98.39`,
  `agent/src/data_uploads/upload/.covgate` = `96.00`,
  `agent/src/workers/.covgate` = `84.67`.

### The files you will edit

`agent/src/data_uploads/retention/queue.rs` (**170 lines, no in-source `mod tests`** — the
queue's tests live in `agent/tests/data_uploads/retention/queue.rs`). Holds `QueueEntry`
(19-27), `DeleteQueueSnapshot { entries: Vec<QueueEntry> }` (29-32), the
`DeleteQueueSnapshotFile` alias over `SingleThreadStateFile` (42), and `Queue` (44-52).

`Queue`'s API, all post-#206: `new(capacity)`; `from_snapshot(capacity, snapshot_file)`;
`len`; `is_empty`; `async enqueue(Job) -> Result<(), DeleteErr>` (capacity-gated, mints the
`QueueEntry` at 100-103, then persists); `next_ready(now) -> Option<QueueEntry>` (109-114,
returns a **clone**, non-mutating, no persist); `count_ready(now) -> usize` (116-123);
`async remove(id: Uuid) -> Option<QueueEntry>` (125-135, persists) over the private
`remove_impl` (137-140); `async requeue(entry: QueueEntry)` (142-150 — removes by id,
pushes at the tail, persists; deliberately *not* capacity-gated); the private
`async persist()` (152-163), the sole disk writer, which logs a `warn!` and swallows
failures; and `#[cfg(test)] pub(crate) fn entries(&self) -> Vec<Job>` (165-169), which maps
`entry.job.clone()`. There is **no** `pop_front` and no `requeue(Job)`.

`agent/src/data_uploads/retention/deleter.rs` (831 lines, of which 343-831 are the
in-source `mod tests`). Holds `const DEFAULT_QUEUE_CAPACITY: usize = 4096;` (21),
`DeleterArgs { now_fn, queue_capacity, snapshot_file }` (33-47),
`SingleThreadDeleter { queue, now_fn }` (49-52), `enum SweepOutcome` (54-64), the sweep
helpers as associated fns inside `impl SingleThreadDeleter` (66-190), and the actor
(`Command`/`Worker`/`Deleter`/`DeleterExt`). `SweepOutcome` today is
`Retry | Deleted | AlreadyGone | Changed` — there is no `NotDue`, because #206 made
readiness the queue's concern. The bug lives in `sweep` (86-104):

    for _ in 0..self.queue.count_ready(now) {
        let Some(entry) = self.queue.next_ready(now) else {
            break;
        };
        match Self::sweep_entry(&entry.job).await {
            SweepOutcome::Retry => self.queue.requeue(entry).await,
            SweepOutcome::Deleted | SweepOutcome::AlreadyGone | SweepOutcome::Changed => {
                self.queue.remove(entry.id).await;
            }
        }
    }

`SweepOutcome::Retry` is produced at exactly three sites, each a `warn!` ending in
"retrying next sweep": `stat_file` (`deleter.rs:132`, any `FileSysErr` from
`files::metadata` other than `PathDoesNotExistErr`), `check_digest_mismatch`
(`deleter.rs:167`, any error from `files::hash`), and `delete_file` (`deleter.rs:186`, any
error from `files::delete`).

`agent/src/data_uploads/retention/mod.rs:10` already re-exports
`{DeleteQueueSnapshot, DeleteQueueSnapshotFile, Queue, QueueEntry}` publicly, so
`QueueEntry` is reachable from the integration-test crate as
`miru_agent::data_uploads::retention::QueueEntry`. **No new re-export is needed.**

`agent/tests/data_uploads/retention/queue.rs` (456 lines) is the queue's test file.
Helpers: `now()` (timestamp 2000), `make_job(name, observed_secs, ttl_secs)` (synthetic —
no file is created on disk; fixed size 42, digest `sha256:{name}`), `open(path)`,
`drain(&mut queue)`, `on_disk(path)`. Test modules: `from_snapshot`, `enqueue`,
`next_ready`, `count_ready`, `remove`, `requeue`, `durability`.
`agent/tests/data_uploads/retention/mod.rs` is `pub mod deleter; pub mod queue; pub mod sink;`
— no registration change is needed for anything in this plan.

### The precedent you are mirroring

`agent/src/data_uploads/upload/`. `queue.rs` defines `QueueEntry` with an `attempts: u32`.
`uploader.rs:266-288` (`handle_counted_failure`) is the policy this plan copies: increment,
`warn!`, drop on terminal, drop when `attempts >= options.attempts`, otherwise requeue.
`uploader.rs:363-368` / `:383` hold the `error!` drop loggers. Do **not** copy
`next_attempt_at`, `requeue_after`, `cooldown::Backoff` or `max_job_age` — see D5.

### What the error types give you

`agent/src/filesys/errors.rs` defines `FileSysErr`, a large `thiserror` enum. The variants
the sweep can hit, and their `io::Error` field:

- `files::metadata` → `PathDoesNotExistErr` (NotFound, already handled as `AlreadyGone`)
  or `FileMetadataErr { file, source: Box<std::io::Error>, trace }` (188-196).
- `files::hash` → `PathDoesNotExistErr` / `OpenFileErr { source, file, trace }` (267-275)
  from the open, or `ReadFileErr { source, file, trace }` (287-295) from the read.
- `files::delete` → `Ok(())` on NotFound (it swallows it), else
  `DeleteFileErr { source, file, trace }` (178-186).
- `PathExistsErr { path, trace }` (53-60) carries **no** `io::Error` — it is the handy
  witness for the `_ => None` arm of `io_kind`.

All four `source` fields are `pub`, so `err.source.kind()` is directly available. The
toolchain is pinned at `1.97.0` (`rust-toolchain.toml`), on which `IsADirectory`,
`NotADirectory`, `ReadOnlyFilesystem` and `InvalidFilename` are stable and matchable by
name — those are the four this plan lists as terminal (alongside `PermissionDenied`).

**Not** every `ErrorKind` is nameable on stable: `FilesystemLoop` (ELOOP) is still behind
the unstable `io_error_more` feature (rust-lang issue #86442) on 1.97.0, so writing
`std::io::ErrorKind::FilesystemLoop` anywhere — including in a test — is a hard compile
error (E0658). That is not a problem for this plan: ELOOP has no nameable variant, so it
lands in `classify`'s `_ =>` arm and is treated as a counted retry, which is exactly the
behavior the counted-retry tests want. Assert on the *classification*, never on the
variant name.

### How delete jobs are minted

There are **two** producers today, and they are disjoint by `retention.require_upload`.
Both feed the single `Queue`, so both inherit the attempt budget this plan adds; there is
no per-producer policy.

1. **The stability sink.** The scanner (`agent/src/data_uploads/scan/`) emits a stable file
   and `scan/scanner.rs:116-118` fans it out to sinks;
   `RetentionStableFileSink::on_stable_file` (`agent/src/data_uploads/retention/sink.rs:33-68`)
   turns it into a `Job` and calls `deleter.enqueue`. It handles only rules whose
   `retention.require_upload` is **false** — a `require_upload: true` rule is explicitly
   `debug!`-skipped at `sink.rs:43-50`.
2. **The uploader, at confirmation.** Landed in #207.
   `Uploader::enqueue_delete_job` (`agent/src/data_uploads/upload/uploader.rs:217-240`) is
   called on `AttemptOutcome::Succeeded` (`uploader.rs:203-209`) for an upload job that
   carries a `retention`, and mints a `retention::Job` stamping `last_observed_at` from the
   confirm instant, so `due_at = confirm + ttl_secs`.

**Neither producer re-fires for an unchanged file**, which is the evidence behind D7:

- The sink is downstream of the scanner's ledger. `RuleState::is_latest_ledger_entry`
  (`agent/src/data_uploads/scan/state.rs:59`) matches the last ledger entry by size +
  mtime; `differs_from_previous` (`scan/rule.rs:272-282`) returns `Outcome::AlreadyInLedger`
  on a digest match; and the ledger persists in the scanner snapshot. An unchanged file is
  never re-emitted, so the sink never re-mints its job.
- The confirm-time producer fires exactly once per successful upload: immediately after
  `enqueue_delete_job`, the uploader removes the upload job from its own queue
  (`uploader.rs:207`), and that upload job itself came from the same non-re-emitting
  scanner stream.

So a dropped delete job is gone for good and the file is a permanent, silent leak — which
is why the drop must be an `error!` and not a `warn!`.

## Plan of Work

### M1 — attempts, classification, drop-on-exhaustion

**In `agent/src/data_uploads/retention/queue.rs`:**

- Add one field to the existing `QueueEntry` (19-27):

      /// Sweeps that failed in a way we chose to count (see `SweepOutcome` in
      /// `deleter.rs`). Defaulted so a snapshot written before this field
      /// existed loads with a full budget rather than resetting the queue.
      #[serde(default)]
      pub attempts: u32,

  Nothing else about the struct, the snapshot, or the persist path changes — the snapshot
  is already `Vec<QueueEntry>` and every mutator already persists.
- `enqueue` mints `attempts: 0` alongside the fresh id (`queue.rs:100-103`).
- Keep `#[cfg(test)] pub(crate) fn entries(&self) -> Vec<Job>` exactly as it is: nine
  assertions in `deleter.rs`'s `mod tests` are written against it. Add beside it

      /// The queued entries, oldest enqueue first (test observability only).
      #[cfg(test)]
      pub(crate) fn queue_entries(&self) -> Vec<QueueEntry> {
          self.entries.iter().cloned().collect()
      }

  so the deleter's unit tests can read `attempts` without the queue's tests having to.

**In `agent/src/data_uploads/retention/deleter.rs`:**

- Add `const DEFAULT_ATTEMPTS: u32 = 10;` next to `DEFAULT_QUEUE_CAPACITY` (21).
- Add to `DeleterArgs` (33-37):

      /// Total sweep attempts per job before it is dropped.
      pub attempts: u32,

  and `attempts: DEFAULT_ATTEMPTS` in its `Default` impl (39-47). Store it on
  `SingleThreadDeleter` (49-52) as a new `attempts: u32` field, set in `new` from
  `args.attempts`.
- Replace the `Retry` variant of `SweepOutcome` (54-64):

      /// A failure whose class might resolve on its own; consumes one attempt.
      CountedRetry,
      /// A failure whose class will never resolve for this recorded path;
      /// drop immediately rather than burning the whole attempt budget.
      TerminalFailure,

- Add the classifier as two private associated functions **inside `impl
  SingleThreadDeleter`**, next to `stat_file`/`check_file_identity`/`delete_file`
  (66-190), so every call site and test reaches them as `Self::io_kind` / `Self::classify`
  (from `mod tests`: `SingleThreadDeleter::classify`):

      /// The `io::ErrorKind` behind a filesystem error, when one is available.
      fn io_kind(err: &FileSysErr) -> Option<std::io::ErrorKind> {
          match err {
              FileSysErr::FileMetadataErr(e) => Some(e.source.kind()),
              FileSysErr::OpenFileErr(e) => Some(e.source.kind()),
              FileSysErr::ReadFileErr(e) => Some(e.source.kind()),
              FileSysErr::DeleteFileErr(e) => Some(e.source.kind()),
              _ => None,
          }
      }

      /// Failures that cannot succeed on a later sweep for the same recorded
      /// path. Everything else — including errors we cannot classify — is
      /// counted, and the attempt cap is the backstop.
      fn classify(err: &FileSysErr) -> SweepOutcome {
          use std::io::ErrorKind::*;
          match Self::io_kind(err) {
              Some(PermissionDenied | ReadOnlyFilesystem | IsADirectory | NotADirectory
                   | InvalidFilename) => SweepOutcome::TerminalFailure,
              _ => SweepOutcome::CountedRetry,
          }
      }

  Keep `use std::io::ErrorKind::*;` scoped inside `classify` so it does not leak into
  the impl or module namespace. Both are plain `fn`, take no `self`, and are called
  `Self::classify(&err)` from the sweep helpers and
  `SingleThreadDeleter::classify(&err)` / `SingleThreadDeleter::io_kind(&err)` from
  `mod tests`.

- The three `Retry` sites (`deleter.rs:132`, `:167`, `:186`) return
  `Self::classify(&err)` instead of `SweepOutcome::Retry`, and keep their existing `warn!`
  — but reword the tail, since "retrying next sweep" is now a lie for the terminal case:
  end each with "; classifying failure" and let the sweep loop do the outcome logging.

- Rewrite the `sweep` match (86-104). The selected entry must now be **mutable**, so the
  `let`-else binds `mut entry`; everything else about the loop, including the
  `count_ready` budget and the `else { break }`, is unchanged:

      for _ in 0..self.queue.count_ready(now) {
          let Some(mut entry) = self.queue.next_ready(now) else {
              break;
          };
          match Self::sweep_entry(&entry.job).await {
              SweepOutcome::CountedRetry => {
                  entry.attempts += 1;
                  if entry.attempts >= self.attempts {
                      Self::log_exhausted_drop(&entry);
                      self.queue.remove(entry.id).await;
                  } else {
                      warn!(
                          "delete: attempt {} of {} failed for {}; retrying next sweep",
                          entry.attempts, self.attempts, entry.job.file
                      );
                      self.queue.requeue(entry).await;
                  }
              }
              SweepOutcome::TerminalFailure => {
                  Self::log_terminal_drop(&entry);
                  self.queue.remove(entry.id).await;
              }
              SweepOutcome::Deleted | SweepOutcome::AlreadyGone | SweepOutcome::Changed => {
                  self.queue.remove(entry.id).await;
              }
          }
      }

  Two properties to preserve and to state in review: (a) `next_ready` returns a *clone*, so
  the increment only reaches disk via `requeue`, which replaces the entry under its id and
  persists (D6); (b) the budget still holds — a counted retry moves the entry to the tail,
  behind every not-yet-visited due entry, so each due entry is visited exactly once, and a
  drop merely shrinks the due set, which the `else { break }` absorbs.

- Add the two `error!` loggers, mirroring `log_dropped` / `log_terminal_drop` in
  `upload/uploader.rs:363-368` and `:383`:

      fn log_exhausted_drop(entry: &QueueEntry) {
          error!(
              "delete: giving up on {} after {} attempts (rule {}, deployment {}, \
               digest {}); the file is left on disk and the agent will not retry it",
              entry.job.file, entry.attempts, entry.job.file_rule_id,
              entry.job.deployment_id, entry.job.digest
          );
      }

      /// `attempts` counts *consumed* attempts and the terminal path deliberately
      /// consumes none, so log the ordinal of the sweep that failed —
      /// `attempts + 1` — not the field. Without the `+ 1` the headline case (a
      /// terminal failure on the very first sweep) logs "attempt 0".
      fn log_terminal_drop(entry: &QueueEntry) {
          error!(
              "delete: giving up on {} on attempt {} after a permanent filesystem \
               failure (rule {}, deployment {}, digest {}); the file is left on \
               disk and the agent will not retry it",
              entry.job.file, entry.attempts + 1, entry.job.file_rule_id,
              entry.job.deployment_id, entry.job.digest
          );
      }

  `deleter.rs` must import `QueueEntry` for these signatures — extend the existing
  `queue::{DeleteQueueSnapshotFile, Queue}` import (10) to include it.
- A successful outcome (`Deleted`, `AlreadyGone`, `Changed`) needs no new code: `remove`
  already drops the entry, attempts and all, and persists.

**Existing tests that must change in M1** (they break at compile or flip meaning the moment
the classifier lands, so they belong in this milestone, not M2):

- `agent/tests/data_uploads/retention/queue.rs:119` — the only literal `QueueEntry { .. }`
  construction outside `Queue::enqueue` anywhere in retention, inside
  `mod from_snapshot :: entry_without_id_gets_one` (114-139). Add `attempts: 0` to it.
  That is the whole compile fallout in the integration crate.
- `mod from_snapshot :: raw_json_snapshot_loads` (85-110) needs **no change**: it writes
  raw JSON with no `attempts` key and asserts on `entry.id` and `entry.job`, never on the
  whole entry, so `#[serde(default)]` keeps it green exactly as written. Confirm this by
  running it rather than by editing it.
- `deleter.rs` `mod sweep :: stat_failure_retains_entry` (606) — ENOTDIR, now terminal.
  Rename to `stat_permanent_failure_drops_entry` and assert `deleter.queue.is_empty()`.
- `deleter.rs` `mod sweep :: hash_failure_retains_entry` (692) — EISDIR, now terminal.
  Rename to `hash_permanent_failure_drops_entry`, same shape; keep `assert!(target.exists())`
  (the directory is untouched).
- `deleter.rs` `mod sweep :: delete_failure_retains_entry` (738) — EISDIR, now terminal.
  Rename to `terminal_failure_drops_job_without_burning_attempts` and rewrite: keep the
  default `attempts: 10`, sweep once, assert `deleter.queue.is_empty()` — the drop happens
  on sweep 1, not sweep 10.

Nothing outside `agent/src/data_uploads/retention/` should need to change: `DeleterExt`,
`Command::Enqueue`, `retention/sink.rs`, `upload/uploader.rs`'s `enqueue_delete_job` and
the app wiring in `agent/src/app/state.rs` all still speak in bare `Job`s, and
`retention/mod.rs` already exports `QueueEntry`. Confirm with a build.

### M2 — tests

All new tests use the existing real-filesystem injection style (no fakes, no mocks).

**Homing rule** (post-#206): queue-level attempt behavior is tested through the *public*
API from `agent/tests/data_uploads/retention/queue.rs`, because `queue.rs` has no in-source
`mod tests`. Deleter-level classification and policy stay in `deleter.rs`'s in-source
`mod tests`, which is where the private `SweepOutcome`/`classify` and the `#[cfg(test)]`
`queue_entries()` accessor are reachable.

**In `agent/tests/data_uploads/retention/queue.rs`:**

- `mod from_snapshot :: entry_without_attempts_defaults_to_zero` — a sibling of
  `entry_without_id_gets_one`, built the same way: serialize a `DeleteQueueSnapshot` with
  one `QueueEntry`, strip the `"attempts"` key from each entry object, write it, load with
  `Queue::from_snapshot`, and assert `queue.next_ready(now()).unwrap().attempts == 0`. This
  is the executable form of D2.
- `mod enqueue :: new_entries_start_at_zero_attempts` — enqueue a job, assert
  `queue.next_ready(now()).unwrap().attempts == 0`.
- `mod requeue :: preserves_attempts` — enqueue, `next_ready`, bump `attempts` on the
  returned clone, `requeue(entry).await`, assert the queued entry now reads the bumped
  count.
- `mod durability :: attempts_survive_a_reload` — same, then reopen the snapshot with
  `open(&path).await` / `Queue::from_snapshot` and assert the count came back off disk.
  This is the queue-level twin of the deleter-level restart test below.

**In `agent/src/data_uploads/retention/deleter.rs`, `mod tests`:**

Counted failures need a filesystem error whose kind is *not* in the terminal set. A mutual
symlink loop gives ELOOP (`raw_os_error() == Some(40)` on Linux), which is repeatable and
— because ELOOP has no `ErrorKind` variant nameable on stable 1.97.0 — falls into
`classify`'s `_` arm and is therefore counted. Add next to `temp_file`/`make_job`:

    /// Two symlinks pointing at each other. `stat` on either fails with ELOOP,
    /// which classifies as a counted retry rather than a terminal failure.
    /// Built with `std::os::unix::fs::symlink` rather than `files::create_symlink`
    /// because the latter asserts the target exists.
    fn symlink_loop(dir: &Dir) -> File {
        let a = dir.file("loop-a");
        let b = dir.file("loop-b");
        std::os::unix::fs::symlink(b.path(), a.path()).unwrap();
        std::os::unix::fs::symlink(a.path(), b.path()).unwrap();
        a
    }

Note the `&Dir` (not `&crate::filesys::Dir`) signature: the helper is copied verbatim into
`agent/tests/` below, which is a separate integration-test crate where `crate::filesys`
does not resolve. Relying on the imported `Dir` makes the one text work in both places.

Before relying on it, confirm the classification empirically once, over a real
ELOOP-producing `FileSysErr` (stat the loop head through `files::metadata` and unwrap the
error) — assert on the outcome, not on a variant name:

    assert!(matches!(
        SingleThreadDeleter::classify(&err),
        SweepOutcome::CountedRetry
    ));

If some platform maps a symlink loop to a kind that *is* in the terminal set, pick any
other non-terminal error source; the classifier unit tests below do not depend on the
filesystem at all.

**`make_job` cannot be used for symlink-loop jobs.** Both `make_job` helpers (the
`deleter.rs` one at 399-412 and the `agent/tests/data_uploads/retention/deleter.rs` one)
call `files::size(file).await.unwrap()` and `files::hash(file).await.unwrap()`, which
themselves hit ELOOP on the loop head and panic in test setup before the deleter is ever
exercised. Every symlink-loop test must build the `Job` literally instead, modelled on the
existing `stat_failure_retains_entry`:

    let job = Job {
        file: symlink_loop(&dir),
        size: 0,
        digest: "sha256:unused".to_string(),
        mtime: DateTime::from_timestamp(1000, 0).unwrap(),
        first_observed_at: DateTime::from_timestamp(1000, 0).unwrap(),
        last_observed_at: DateTime::from_timestamp(1000, 0).unwrap(),
        ttl_secs: 0,
        file_rule_id: "rule_1".to_string(),
        deployment_id: "dpl_1".to_string(),
    };

The recorded size/digest/mtime are never compared, because the sweep fails at the stat
step before any identity check runs.

The new tests need imports that `mod tests` does not have today (its current block is at
`deleter.rs:350-353`). Extend the internal-crate imports to:

    use super::{DeleterArgs, SingleThreadDeleter, SweepOutcome};
    use crate::data_uploads::retention::job::Job;
    use crate::data_uploads::retention::queue::{DeleteQueueSnapshot, DeleteQueueSnapshotFile};
    use crate::filesys::errors::{
        DeleteFileErr, FileMetadataErr, FileSysErr, OpenFileErr, PathExistsErr, ReadFileErr,
    };
    use crate::filesys::{dirs, files, Dir, File, PathExt, WriteOptions};
    use crate::trace;

CI clippy runs with `-D warnings`, so keep this list to what is actually named: `Dir` is
used by `symlink_loop`'s signature above, and `QueueEntry` is deliberately **not** imported
here — no `deleter.rs` test names the type; they reach the counter through
`deleter.queue.queue_entries()[0].attempts`.

Construct each error struct exactly per its definition in `agent/src/filesys/errors.rs`
(`PathExistsErr { path, trace }` at 53-60; the four io-bearing variants at 178-186,
188-196, 267-275 and 287-295 each carry `source: Box<std::io::Error>` plus a `file` field
and `trace`). `SweepOutcome`, `io_kind` and `classify` are private to the module, which is
fine: `mod tests` is a child module.

New `mod classify` (a sibling of `mod sweep`), constructing `FileSysErr` values directly —
this is where the mapping is pinned precisely and cheaply:

- `permission_denied_is_terminal` — build
  `FileSysErr::DeleteFileErr(DeleteFileErr { source: Box::new(std::io::Error::from(
  std::io::ErrorKind::PermissionDenied)), file: File::new("/data/a.log"), trace: trace!() })`
  and assert `matches!(SingleThreadDeleter::classify(&err), SweepOutcome::TerminalFailure)`.
- `read_only_filesystem_is_terminal`, `is_a_directory_is_terminal`,
  `not_a_directory_is_terminal` — same shape over the other terminal kinds, varying the
  `FileSysErr` variant (`FileMetadataErr`, `ReadFileErr`, `OpenFileErr`) so every arm of
  `io_kind` is exercised.
- `unclassified_io_error_is_counted` — `ErrorKind::Other` (or `TimedOut`) →
  `CountedRetry`.
- `error_without_an_io_source_is_counted` — `FileSysErr::PathExistsErr(..)` →
  `CountedRetry`, pinning the `_ => None` arm.

  `SweepOutcome` needs `#[derive(Debug)]` for `matches!`-adjacent assertion messages; add
  it if the compiler asks.

New tests in `mod sweep`:

- `counted_failure_increments_attempts` — `dirs::temp("delete-attempts-counted")`,
  `symlink_loop(&dir)`, a literal `Job` over the loop head with `ttl_secs: 0`; enqueue,
  sweep once, assert `deleter.queue.queue_entries()[0].attempts == 1` and
  `deleter.queue.len() == 1`; sweep again, assert `attempts == 2`.
- `attempt_cap_drops_job` — same setup but `DeleterArgs { attempts: 3, .. }`; sweep three
  times; after the third assert `deleter.queue.is_empty()`. Uses a small cap so the test is
  three sweeps, not ten.
- `default_attempts_is_ten` — `assert_eq!(DeleterArgs::default().attempts, 10);` plus a
  loop of ten sweeps over the symlink-loop job asserting the queue is non-empty for the
  first nine and empty after the tenth. This is the test that pins the headline number.
- `successful_delete_clears_the_entry` — the persistence-aware sibling of the existing
  `zero_delay_entry_is_deleted_on_first_sweep` (719): build the deleter with a snapshot
  file, enqueue a normal temp-file job, sweep once, assert `deleter.queue.is_empty()` and
  `!tmp.file().exists()`, then drop and rebuild from the snapshot file and assert the
  rebuilt queue is empty — so no orphan attempts record survives a successful delete.

New `mod persistence` in `deleter.rs` `mod tests` (mirroring the existing
`each_drop_is_persisted_before_the_next_job` restart pattern at 764):

- `attempts_survive_a_restart` — `dirs::temp("delete-attempts-restart")`,
  `state_path = dir.file("delete_queue.json")`, deleter built with
  `snapshot_file: Some(snapshot_file(&state_path).await)`; enqueue a symlink-loop job;
  sweep twice; `drop(deleter)`; rebuild with a cap of 3 — the cap must be set on the
  *rebuild*, since `..DeleterArgs::default()` alone would restore the default budget of 10
  and the third sweep would requeue instead of dropping:

      SingleThreadDeleter::new(DeleterArgs {
          attempts: 3,
          snapshot_file: Some(snapshot_file(&state_path).await),
          ..DeleterArgs::default()
      })

  Assert `restored.queue.queue_entries()[0].attempts == 2`, then sweep the restored deleter
  once and assert it drops — the third consumed attempt hits the cap, proving the budget is
  genuinely cumulative across restarts rather than reset by the rebuild. This test is also
  the behavioral pin on D6: it fails if `Queue::requeue` ever stops persisting.
- `dropped_entry_is_absent_from_the_persisted_snapshot` — same setup with
  `DeleterArgs { attempts: 1, .. }`; one sweep; `drop(deleter)`; rebuild; assert
  `restored.queue.is_empty()`. This is the "entry absent from the persisted snapshot"
  requirement, checked through the real file rather than through memory.

**In `agent/tests/data_uploads/retention/deleter.rs`** (integration, through the actor
handle; `agent/tests/data_uploads/retention/mod.rs` already declares `pub mod deleter;`,
so no registration change):

- `wedged_job_is_given_up_on_through_the_actor` — `Deleter::spawn(16, DeleterArgs {
  attempts: 2, ..DeleterArgs::default() })`; enqueue a job over a symlink loop built in a
  `dirs::temp` directory. The `symlink_loop` helper above lives in the `src` `mod tests`
  and is not reachable from `agent/tests/`, so copy it into this file (it is five lines)
  and extend the import at `agent/tests/data_uploads/retention/deleter.rs:3` to
  `use miru_agent::filesys::{dirs, files, Dir, File, PathExt, WriteOptions};` — the `&Dir`
  signature is what makes the copy compile here, since `crate::filesys` names this
  integration-test crate, not the library. This file's `make_job` is unusable for a
  symlink-loop job for the same ELOOP reason as the `src` one, so build the `Job` literally
  as shown above; `Utc::now()` is fine for the three timestamps here, since the actor runs
  on the wall clock rather than an injected one, and `ttl_secs: 0` makes it due at once.
  Then `deleter.sweep()` twice; assert `deleter.len().await.unwrap() == 0`; `shutdown` +
  `handle.await`. This is the only new integration test in `deleter.rs` — it proves the
  policy is reachable through the public `DeleterExt` surface, which is what
  `agent/src/app/state.rs` wires up.

Lint note: any test function with 4 or more `assert_eq!` against fields of one variable
needs `// lint:allow(field-by-field-assert)` inside the body (the CI import/assert linter
flags it). Prefer fewer, more meaningful assertions.

### M3 — validation

Full preflight, push, CI. See "Validation and Acceptance".

## Concrete Steps

Working directory for **every** command below is the repo root:

    /home/ben/miru/workbench1/repos/agent-wt-delete-attempts

Confirm the starting point first:

    git rev-parse --abbrev-ref HEAD     # expect: feat/retention-delete-attempts
    git log --oneline -3                # expect: c32f306, 4b8a2de, 6d1a9b3
    git status --short                  # expect: clean

### M1

1. Edit `agent/src/data_uploads/retention/queue.rs` per M1 above (the `attempts` field, the
   `enqueue` mint, the `queue_entries()` accessor).
2. Edit `agent/src/data_uploads/retention/deleter.rs` per M1 above (`DEFAULT_ATTEMPTS`,
   `DeleterArgs.attempts`, `SingleThreadDeleter.attempts`, the `SweepOutcome` split,
   `io_kind`/`classify`, the three reworded `warn!` sites, the new `sweep` match, the two
   `error!` loggers, the `QueueEntry` import).
3. Update `agent/tests/data_uploads/retention/queue.rs:119` (`attempts: 0`) and rename /
   rewrite the three `deleter.rs` sweep tests whose meaning flipped, per M1's last block.
   No change to `retention/mod.rs` is needed — `QueueEntry` is already re-exported.
4. Compile-check both the library and the tests. A plain `cargo build` skips `#[cfg(test)]`
   blocks and the `agent/tests/` targets entirely, so a rename that misses a test reference
   builds clean locally and then fails CI. Always use `--all-targets`:

       cargo check --package miru-agent --features test --all-targets

5. Run the tests:

       ./scripts/test.sh 2>&1 | tail -40

   The whole retention suite must be green here, including `raw_json_snapshot_loads`
   unedited. Do not commit a red M1.

6. Commit:

       git add agent/src/data_uploads/retention/ agent/tests/data_uploads/retention/
       git commit -m "$(cat <<'EOF'
       feat(retention): give up on delete jobs after 10 failed attempts

       Adds a serde-defaulted `attempts` counter to the delete queue's
       QueueEntry -- backward compatible with snapshots written since the
       {id, job} entry shape landed -- and splits SweepOutcome::Retry into
       CountedRetry and TerminalFailure, classified by io::ErrorKind.
       Permission-denied, read-only-filesystem, EISDIR, ENOTDIR and
       ENAMETOOLONG drop the job immediately; every other failure consumes one
       of DeleterArgs::attempts (default 10). The counter is durable for free:
       Queue::requeue already persists as its last act. Exhaustion and terminal
       drops log at error! naming file, rule, deployment, digest and attempt
       count -- the file is left on disk and the job is never retried, so the
       log line is the only remaining trace.

       Co-Authored-By: Claude <noreply@anthropic.com>
       EOF
       )"

### M2

7. Add the new tests per M2 — `agent/tests/data_uploads/retention/queue.rs` first (public
   API, fastest feedback), then `deleter.rs`'s `mod classify` / `mod sweep` /
   `mod persistence`, then the one integration test in
   `agent/tests/data_uploads/retention/deleter.rs`.
8. Run:

       cargo check --package miru-agent --features test --all-targets
       ./scripts/test.sh

9. Re-ratchet coverage. Do **not** hand-edit `.covgate` values:

        ./scripts/covgate.sh
        ./scripts/update-covgates.sh   # only if covgate.sh reports a module below its gate
        git diff -- '**/.covgate'

    Expect `agent/src/data_uploads/retention/.covgate` to move at most a little from
    `98.39`. If `agent/src/workers/.covgate` shows a local-only failure, leave it alone —
    see the note in Validation.

10. Commit:

        git add agent/src/data_uploads/retention/ agent/tests/data_uploads/retention/ '**/.covgate'
        git commit -m "$(cat <<'EOF'
        test(retention): cover delete attempt accounting and give-up behavior

        Classifier unit tests over constructed FileSysErr values, sweep-level
        tests using a real ELOOP symlink loop for counted failures and EISDIR /
        ENOTDIR for terminal ones, queue-level tests that the attempts field is
        serde-defaulted and survives a snapshot reload, restart tests proving
        the count is cumulative across a rebuild from delete_queue.json, and one
        integration test through the Deleter actor handle.

        Co-Authored-By: Claude <noreply@anthropic.com>
        EOF
        )"

### M3

11. Refresh the lockfile and run the full pre-push gate:

        ./scripts/update-deps.sh
        ./scripts/preflight.sh

    `preflight.sh` runs lint, covgate, tools lint and tools tests in parallel. `lint.sh`
    auto-fixes some findings (`cargo fmt`, `clippy --fix`), so re-check afterwards:

        git status --short

    If it made changes, amend them into the M2 commit (or add a fixup commit — either is
    fine, the branch is not yet pushed):

        git add -A && git commit --amend --no-edit

12. Push and open a draft PR:

        git push -u origin feat/retention-delete-attempts

13. Watch CI to green, then take the PR out of draft. Per the repo's tooling notes, `gh`'s
    GraphQL-backed commands are unreliable here — use the REST API:

        gh api repos/mirurobotics/agent/commits/$(git rev-parse HEAD)/check-runs \
          --jq '.check_runs[] | "\(.name): \(.conclusion // .status)"'

14. If the PR body or state needs editing, use `gh api` REST rather than `gh pr edit`.

## Validation and Acceptance

### Hard gate

`./scripts/preflight.sh` must report **CLEAN**, and **CI must be green on the pushed branch
head**, before the PR leaves draft or the task is reported complete. Both, not either. The
exact commands, from `/home/ben/miru/workbench1/repos/agent-wt-delete-attempts`:

    ./scripts/update-deps.sh
    ./scripts/preflight.sh
    git push -u origin feat/retention-delete-attempts
    gh api repos/mirurobotics/agent/commits/$(git rev-parse HEAD)/check-runs \
      --jq '.check_runs[] | "\(.name): \(.conclusion // .status)"'

Known non-signal: `agent/src/workers/.covgate` (currently `84.67`) fails locally on
branches that do not touch `agent/src/workers/` at all — a pre-existing local-vs-CI
coverage gap. Do not chase it, do not ratchet it down, and do not let it block the branch.
If `covgate.sh` fails *only* on `agent/src/workers`, treat preflight as clean for the
purposes of this plan and say so in the PR description.

### Behavioral acceptance

Each of these is a behavior a human can verify by running the suite and reading the test
names, or by reading the log output of a running agent:

1. A counted failure consumes one attempt per sweep — `counted_failure_increments_attempts`.
2. Ten attempts is the give-up point — `default_attempts_is_ten` and `attempt_cap_drops_job`.
3. Exhaustion removes the entry from disk, not just memory —
   `dropped_entry_is_absent_from_the_persisted_snapshot`.
4. A terminal failure does not burn the budget —
   `terminal_failure_drops_job_without_burning_attempts`.
5. The attempt count is cumulative across restarts — `attempts_survive_a_restart`, backed
   at the queue level by `attempts_survive_a_reload`.
6. A successful delete clears queue, file and snapshot — `successful_delete_clears_the_entry`.
7. The actor surface honors the policy — `wedged_job_is_given_up_on_through_the_actor`,
   driven through `DeleterExt`, the same API `agent/src/app/state.rs` wires up.
8. Existing snapshots keep loading — `entry_without_attempts_defaults_to_zero`, plus
   `raw_json_snapshot_loads` still passing **unedited**.
9. **The give-up is loud.** Run the agent (or grep the sources) and confirm the drop paths
   are `error!`, name the file path, `file_rule_id`, `deployment_id`, `digest` and the
   attempt count, and say the file is left on disk. The terminal drop reports
   `attempts + 1` (the ordinal of the failing sweep), so a first-sweep terminal drop reads
   "on attempt 1", never "attempt 0". A `warn!` here would be a defect: the log line is the
   only surviving record of a leaked file.

## Idempotence and Recovery

- Every step is a source edit plus a rerun of a script; all are safe to repeat.
  `./scripts/test.sh`, `./scripts/covgate.sh`, `./scripts/lint.sh` and
  `./scripts/preflight.sh` are read-only with respect to your intent — `lint.sh` mutates
  source only by auto-formatting and applying clippy fixes, which is idempotent; always
  re-run `git status --short` after it.
- `./scripts/update-covgates.sh` rewrites `.covgate` files in place. Re-running is safe.
  If it ratchets a module you did not touch, `git checkout -- <path>/.covgate` that file.
- **There is no risky on-disk migration in this plan** (D2): adding a `#[serde(default)]`
  field to an entry shape that already exists is backward compatible, so a live agent's
  `delete_queue.json` keeps loading with every entry at `attempts: 0`. No on-disk test
  fixture is shared across tests either — each uses its own `dirs::temp`. To prove the load
  on a live agent root anyway, back the file up first (the snapshot is
  `<filesystem_root>/var/lib/miru/delete_queue.json`; see `agent/src/disk/layout.rs:14-19,
  53-55`).

- To abandon the *implementation* at any point before pushing and return to the branch head
  as it stands before M1 — `c32f306`, the latest plan commit, which contains this document
  and no implementation:

      git reset --hard c32f306

  Do **not** reset to `6d1a9b3`: that is `main`, and it predates this plan file, so the
  reset deletes the document you are executing. (It is recoverable via `git reflog`, but
  do not rely on that.) After pushing, prefer a revert commit over a force-push.
- If `./scripts/test.sh` fails only in `agent/src/data_uploads/retention/` partway through
  M1, that is the expected mid-milestone state for the three renamed EISDIR/ENOTDIR tests
  and the `attempts: 0` literal; finish step 3 before concluding anything is wrong.

## Outcomes & Retrospective

_(filled at completion)_
