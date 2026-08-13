# Retention delete worker — attempt accounting (give up after 10 attempts)

## Scope

| Repo | Path | Access | Notes |
|------|------|--------|-------|
| agent | /home/ben/miru/workbench1/repos/agent-wt-delete-attempts | read-write | All changes in crate `miru-agent`, module `agent/src/data_uploads/retention/` |

Branch `feat/retention-delete-attempts`, based on `main` at `95bc2a5`. No other repo is
read or written. No API spec, no infra, no backend change.

## Purpose / Big Picture

The retention delete worker (`agent/src/data_uploads/retention/`) keeps a persisted queue
of *delete jobs*: records saying "file X, whose size/mtime/digest were Y, may be deleted
after time T". A background driver sweeps the queue every 60 seconds; each due job is
re-stat'd, its identity re-checked, and the file unlinked.

Today a job whose sweep fails — a stat error, a hash error, an unlink error — is pushed
back on the queue with no memory that it failed. It will be retried every 60 seconds
**forever**, logging a `warn!` each time. A file sitting on a read-only mount, or a path
that has become a directory, produces one warning per minute for the life of the process
and never resolves.

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

**The dropped job is not retried and the file is not deleted.** That is deliberate and is
the single most important thing to understand about this change; see "Why dropping is safe
(and what it leaks)" below.

## Progress

- [ ] M1: `DeleteQueueEntry { job, attempts }` in `retention/queue.rs`; snapshot shape change; deleter plumbed through the new entry type; `DEFAULT_ATTEMPTS` + `DeleterArgs.attempts`
- [ ] M2: Split `SweepOutcome::Retry` into counted-retry vs terminal; classify `FileSysErr` by `io::ErrorKind`; drop on exhaustion / terminal with `error!`; persist on the counted-retry path
- [ ] M3: Tests (in-source unit tests + integration test); covgate re-ratchet
- [ ] M4: `./scripts/preflight.sh` clean, push, CI green, PR out of draft

## Surprises & Discoveries

_(filled as work proceeds)_

## Decision Log

- **D1 — attempts live on a new queue entry, not on `Job`** (2026-08-12, plan author).
  Introduce `pub struct DeleteQueueEntry { pub job: Job, pub attempts: u32 }` in
  `agent/src/data_uploads/retention/queue.rs` and change
  `DeleteQueueSnapshot.entries` from `Vec<Job>` to `Vec<DeleteQueueEntry>`. This mirrors
  `agent/src/data_uploads/upload/queue.rs:19-29` (`QueueEntry { id, job, attempts,
  next_attempt_at }`). `Job` stays the pure domain record minted by
  `retention/sink.rs` and compared whole in tests; bookkeeping does not belong on it.
  Deliberately **no `id: Uuid`** field, unlike upload: upload needs an id because
  `next_ready` returns a *clone* and the entry is later located by id; retention's sweep
  pops each entry into ownership and pushes it back, so there is nothing to look up. Also,
  same-path duplicate jobs are explicitly allowed and tested
  (`duplicate_same_path_jobs_are_both_queued`, `same_path_duplicate_resolves_in_one_sweep`),
  so an identity field would be gratuitous divergence.

- **D2 — the snapshot shape change resets any existing on-disk queue** (2026-08-12).
  `delete_queue.json` currently serialises as `{"entries":[<Job>, ...]}` and will serialise
  as `{"entries":[{"job":<Job>,"attempts":0}, ...]}`. An old file therefore fails to parse,
  and `SingleThreadStateFile::new_with_default`
  (`agent/src/filesys/state_file.rs:54-63`) falls back to writing the default — an empty
  queue. Accepted: there are zero production users, the loss is fail-open (files are
  *retained*, never wrongly deleted), and the umbrella plan already sanctions
  reset-on-parse-failure for persisted-state shape changes. `#[serde(default)]` on
  `attempts` is still added (cheap forward-compat for the *next* shape change) but does not
  rescue the old shape, because `job` would be missing.

- **D3 — the limit is a documented option with a named default, not a bare literal**
  (2026-08-12). `DeleterArgs` (`retention/deleter.rs:33-47`) is the only knob struct the
  deleter has — there is no `DeleterOptions` — so the field goes there:
  `pub attempts: u32`, documented "Total sweep attempts per job before it is dropped.",
  defaulted from a new `const DEFAULT_ATTEMPTS: u32 = 10;` next to the existing
  `const DEFAULT_QUEUE_CAPACITY: usize = 4096;`. This mirrors
  `UploaderOptions.attempts: u32` (default 30) in `upload/uploader.rs:34-73`.

- **D4 — classification is a split of `SweepOutcome`, driven by `io::ErrorKind`**
  (2026-08-12). `SweepOutcome::Retry` becomes two variants, `CountedRetry` and
  `TerminalFailure`, decided at the three producing sites. `FileSysErr` already exposes
  enough to discriminate — every relevant variant carries a public
  `source: Box<std::io::Error>` — so **no new predicate on `FileSysErr` is added** (the
  task allowed adding one; it is unnecessary). A private helper in `retention/deleter.rs`
  extracts the kind. Terminal kinds: `PermissionDenied`, `ReadOnlyFilesystem`,
  `IsADirectory`, `NotADirectory`, `InvalidFilename`. Everything else — including any
  `FileSysErr` variant with no extractable `io::Error` — is counted, on the principle that
  the attempt cap is precisely the backstop for failures we cannot classify. A vanished
  file is *not* a failure at all: `files::metadata` maps `NotFound` to
  `FileSysErr::PathDoesNotExistErr` and the existing code already returns
  `SweepOutcome::AlreadyGone` for it. This is the lesson of the upload side's
  `is_terminal`/`is_network_conn_err` bools (`upload/errors.rs:17-34`): state deliberately
  which classes consume budget, and log the rest loudly.

- **D5 — no per-job exponential backoff** (2026-08-12). Deliberate divergence from upload.
  `agent/src/workers/delete.rs` already paces the retries at a fixed
  `Options.sweep_interval_secs` (default 60), so 10 attempts is ≈10 minutes of retrying and
  a `next_attempt_at` field would only add state without changing the cadence. Upload needs
  backoff because its executor is driven by network work, not by a fixed sweep. Adding
  `cooldown::Backoff` (`agent/src/cooldown/mod.rs`) is a clean follow-up if 10 minutes
  proves too short.

- **D6 — the counted-retry path must persist** (2026-08-12). Today `SweepOutcome::NotDue |
  Retry` requeues without persisting. If the incremented counter is not written to disk, it
  resets on every restart and the cap never fires. So the counted-retry branch calls
  `self.queue.persist().await` after requeueing. `NotDue` keeps its non-persisting requeue
  (nothing changed). Write-volume implication: one snapshot write per *failing* job per
  sweep, bounded by the 60s sweep interval and by the fact that a failing job is dropped
  after 10 sweeps.

- **D7 — exhaustion drops the job and leaks the file, loudly** (2026-08-12). On the 10th
  counted failure, or on the first terminal failure, the entry is *not* requeued; the queue
  is persisted and an `error!` is emitted naming file path, `file_rule_id`,
  `deployment_id`, `digest` and attempt count — mirroring `log_dropped` /
  `log_terminal_drop` in `upload/uploader.rs:330-357`. Nothing extra is persisted to stop
  the job coming back, because nothing brings it back: the scanner's
  `RuleState::is_latest_ledger_entry` (`agent/src/data_uploads/scan/state.rs:59-64`)
  suppresses re-emission of an unchanged file, and `prune_ledger` only forgets files that
  have disappeared from the glob. So the job is gone for good and the file stays on disk,
  forgotten — fail-open, never fail-deadly. Corollary, and this is correct behavior rather
  than a loop: if the file is later **modified**, the scanner legitimately re-emits it as a
  new stable file, the sink mints a fresh `Job`, and it gets a fresh budget of 10.

## Context and Orientation

Read this section as if you have never opened this repository. Every path below is
relative to the repo root `/home/ben/miru/workbench1/repos/agent-wt-delete-attempts`.

### Terms

- **Delete job** — `agent/src/data_uploads/retention/job.rs`, `pub struct Job`. Public
  fields: `file: File`, `size: u64`, `digest: String`, `mtime`, `first_observed_at`,
  `last_observed_at: DateTime<Utc>`, `ttl_secs: u64`, `file_rule_id: String`,
  `deployment_id: String`. Derives `Clone, Debug, PartialEq, Serialize, Deserialize`.
  `Job::due_at()` returns `last_observed_at + ttl_secs`, saturating to
  `DateTime::<Utc>::MAX_UTC` ("never due") on overflow. There is no id and no attempt
  count — that is what this plan adds, one level up.
- **Sweep** — one pass of `SingleThreadDeleter::sweep` over the queue. It reads the queue
  length `n`, then loops `n` times: pop the front entry into ownership, decide, and either
  push it back at the tail or drop it. Popping into ownership is what makes a drop
  unambiguous, and requeueing at the tail is what makes each entry considered exactly once
  per sweep.
- **Sweep driver** — `agent/src/workers/delete.rs`. `Options { sweep_interval_secs: i64 }`,
  default 60. Runs one sweep immediately at boot, then sleeps and sweeps forever. It is the
  only thing that imposes a cadence; the deleter actor is purely reactive.
- **Snapshot** — `delete_queue.json`, path from `agent/src/disk/layout.rs:53`
  (`delete_queue()` = `root()/delete_queue.json`). Written through
  `SingleThreadStateFile` (`agent/src/filesys/state_file.rs`), which caches state in memory
  and writes atomically. `new_with_default` writes the default on *any* read or parse
  failure — see D2.
- **Covgate** — a per-module `.covgate` file holding a minimum line-coverage percentage,
  enforced by `./scripts/covgate.sh`. Current values:
  `agent/src/data_uploads/retention/.covgate` = `98.39`,
  `agent/src/data_uploads/upload/.covgate` = `96.00`,
  `agent/src/workers/.covgate` = `84.67`.

### The files you will edit

`agent/src/data_uploads/retention/queue.rs` (382 lines). Holds:

    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
    pub struct DeleteQueueSnapshot {
        pub entries: Vec<Job>,
    }

    pub type DeleteQueueSnapshotFile = SingleThreadStateFile<DeleteQueueSnapshot, DeleteQueueSnapshot>;

    pub struct Queue {
        entries: VecDeque<Job>,
        capacity: usize,
        snapshot_file: Option<DeleteQueueSnapshotFile>,
    }

`Queue`'s API: `new(capacity)`, `from_snapshot(capacity, snapshot_file)`, `len`,
`is_empty`, `enqueue(Job) -> Result<(), DeleteErr>` (returns `QueueFullErr` at capacity),
`pop_front() -> Option<Job>`, `requeue(Job)` (pushes at the tail, deliberately *not*
capacity-gated), `async persist()` (the sole disk writer; logs a `warn!` and swallows
failures), and `#[cfg(test)] pub(crate) fn entries(&self) -> Vec<Job>`.

`agent/src/data_uploads/retention/deleter.rs` (801 lines, of which ~450 are in-source
tests). Holds `const DEFAULT_QUEUE_CAPACITY: usize = 4096;`, `DeleterArgs`,
`SingleThreadDeleter { queue, now_fn }`, the `SweepOutcome` enum, the actor
(`Command`/`Worker`/`Deleter`/`DeleterExt`). The bug lives in `sweep`:

    match Self::sweep_entry(&entry, now).await {
        SweepOutcome::NotDue | SweepOutcome::Retry => {
            self.queue.requeue(entry);
        }
        SweepOutcome::Deleted | SweepOutcome::AlreadyGone | SweepOutcome::Changed => {
            self.queue.persist().await;
        }
    }

`SweepOutcome::Retry` is produced at exactly three sites, each a `warn!` followed by
`Retry`:

1. `stat_file` — any `FileSysErr` other than `PathDoesNotExistErr` from `files::metadata`.
2. `check_digest_mismatch` — any error from `files::hash`.
3. `delete_file` — any error from `files::delete`.

`agent/src/data_uploads/retention/mod.rs` re-exports `DeleteQueueSnapshot` and
`DeleteQueueSnapshotFile`; add `DeleteQueueEntry` there too.

### The precedent you are mirroring

`agent/src/data_uploads/upload/`. `queue.rs:19-29` defines `QueueEntry` with an `attempts:
u32`. `uploader.rs:234-256` (`handle_counted_failure`) is the policy this plan copies:
increment, `warn!`, drop on terminal, drop when `attempts >= options.attempts`, otherwise
requeue. `uploader.rs:330-357` holds the three `error!` drop loggers. Do **not** copy
`next_attempt_at`, `earliest_next_attempt`, `cooldown::Backoff` or `max_job_age` — see D5.

### What the error types give you

`agent/src/filesys/errors.rs` defines `FileSysErr`, a large `thiserror` enum. The variants
the sweep can hit, and their `io::Error` field:

- `files::metadata` → `PathDoesNotExistErr` (NotFound, already handled as `AlreadyGone`)
  or `FileMetadataErr { file, source: Box<std::io::Error>, trace }`.
- `files::hash` → `PathDoesNotExistErr` / `OpenFileErr { source: Box<io::Error>, .. }` from
  the open, or `ReadFileErr { source: Box<io::Error>, .. }` from the read.
- `files::delete` → `Ok(())` on NotFound (it swallows it), else
  `DeleteFileErr { source: Box<io::Error>, .. }`.

All four `source` fields are `pub`, so `err.source.kind()` is directly available. The
toolchain is pinned at `1.97.0` (`rust-toolchain.toml`), so the `io_error_more` kinds
(`IsADirectory`, `NotADirectory`, `ReadOnlyFilesystem`, `InvalidFilename`,
`FilesystemLoop`, …) are stable and matchable.

### Why dropping is safe (and what it leaks)

Delete jobs are minted in exactly one place today: the scanner
(`agent/src/data_uploads/scan/`) emits a stable file, `scan/scanner.rs:107` fans it out to
sinks, and `retention/sink.rs` (`RetentionStableFileSink::on_stable_file`) turns it into a
`Job` and calls `deleter.enqueue`. Re-minting the *same* job is suppressed three ways:
`RuleState::is_latest_ledger_entry` (`scan/state.rs:59-64`) matches the last ledger entry
by size + mtime; `differs_from_previous` returns `Outcome::AlreadyInLedger` on a digest
match (`scan/rule.rs:257-276`); and the ledger persists in the scanner snapshot.

So there is **no resurrection loop** to worry about. The hazard is the opposite one:
dropping the job means the agent forgets a file it never managed to delete, and the file
stays on disk. That is why the drop must be `error!`, not `warn!` or `debug!` — the log
line is the only remaining trace. This is the same failure mode that commits `370483b`
(#185) and `5a6cae2` (#204) addressed on the upload side.

Note also that PR 3a milestone M2 has not landed: `LiveExecutor::delete_source_file`
(`agent/src/data_uploads/upload/executor.rs:87-105`) still deletes inline for
`retention == Some({require_upload: true, ttl_secs: 0})`. A second minting path is coming
and will inherit whatever semantics this plan establishes.

## Plan of Work

### M1 — queue entry and attempts plumbing

In `agent/src/data_uploads/retention/queue.rs`:

- Add, above `DeleteQueueSnapshot`:

      /// A queued delete job plus its bookkeeping. `attempts` counts sweeps that
      /// failed in a way we chose to count (see `SweepOutcome` in `deleter.rs`);
      /// it is persisted so the budget survives a restart.
      #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
      pub struct DeleteQueueEntry {
          pub job: Job,
          #[serde(default)]
          pub attempts: u32,
      }

- Change `DeleteQueueSnapshot.entries` to `Vec<DeleteQueueEntry>` and
  `Queue.entries` to `VecDeque<DeleteQueueEntry>`.
- `enqueue(job: Job)` keeps its signature (the sink and the actor `Command::Enqueue` pass a
  bare `Job`) and wraps internally: `self.entries.push_back(DeleteQueueEntry { job,
  attempts: 0 })`. The capacity check reads `job.file` before the wrap, unchanged.
- `pop_front` returns `Option<DeleteQueueEntry>`; `requeue` takes `DeleteQueueEntry`.
- `persist` clones `DeleteQueueEntry` values into the snapshot; body otherwise unchanged.
- Keep `#[cfg(test)] pub(crate) fn entries(&self) -> Vec<Job>` returning
  `self.entries.iter().map(|e| e.job.clone()).collect()`, so the ~10 existing assertions of
  the form `assert_eq!(queue.entries(), [job])` keep compiling and keep meaning what they
  meant. Add alongside it
  `#[cfg(test)] pub(crate) fn queue_entries(&self) -> Vec<DeleteQueueEntry>` for the new
  attempt assertions, and `#[cfg(test)] pub(crate) fn attempts(&self) -> Vec<u32>` if it
  reads better at the call sites.

In `agent/src/data_uploads/retention/deleter.rs`:

- Add `const DEFAULT_ATTEMPTS: u32 = 10;` next to `DEFAULT_QUEUE_CAPACITY`.
- Add to `DeleterArgs`:

      /// Total sweep attempts per job before it is dropped.
      pub attempts: u32,

  and `attempts: DEFAULT_ATTEMPTS` in its `Default` impl. Store it on
  `SingleThreadDeleter` as a new `attempts: u32` field, set in `new` from `args.attempts`.
- Adjust `sweep` for the new pop/requeue types: the loop now binds
  `let Some(entry) = self.queue.pop_front()` where `entry: DeleteQueueEntry`, and
  `sweep_entry` takes `&entry.job`.

In `agent/src/data_uploads/retention/mod.rs`, add `DeleteQueueEntry` to the `pub use
self::queue::{...}` list.

Nothing outside `agent/src/data_uploads/retention/` should need to change; `DeleterExt`,
`Command::Enqueue`, `retention/sink.rs` and the app wiring in `agent/src/app/state.rs`
still speak in bare `Job`s. Confirm with a build.

### M2 — classification, drop-on-exhaustion, logging

All in `agent/src/data_uploads/retention/deleter.rs`.

- Replace the `Retry` variant of `SweepOutcome`:

      /// A failure whose class might resolve on its own; consumes one attempt.
      CountedRetry,
      /// A failure whose class will never resolve for this recorded path;
      /// drop immediately rather than burning the whole attempt budget.
      TerminalFailure,

- Add the classifier, private to this module:

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
          match io_kind(err) {
              Some(PermissionDenied | ReadOnlyFilesystem | IsADirectory | NotADirectory
                   | InvalidFilename) => SweepOutcome::TerminalFailure,
              _ => SweepOutcome::CountedRetry,
          }
      }

  Keep `use std::io::ErrorKind::*;` scoped inside the function so it does not leak into the
  module namespace.

- The three `Retry` sites call `Self::classify(&err)` instead of returning `Retry`, and
  keep their existing `warn!` (the message tail "retrying next sweep" is now a lie for the
  terminal case — reword to "; classifying failure" and let the sweep loop do the outcome
  logging).

- Rewrite the `sweep` match:

      match Self::sweep_entry(&entry.job, now).await {
          SweepOutcome::NotDue => {
              self.queue.requeue(entry);
          }
          SweepOutcome::CountedRetry => {
              self.record_failed_attempt(entry).await;
          }
          SweepOutcome::TerminalFailure => {
              Self::log_terminal_drop(&entry);
              self.queue.persist().await;
          }
          SweepOutcome::Deleted | SweepOutcome::AlreadyGone | SweepOutcome::Changed => {
              self.queue.persist().await;
          }
      }

  Note the terminal and exhausted branches simply do not requeue — the entry was already
  popped, so persisting the shortened queue is the drop.

- Add the counted-failure handler, mirroring `handle_counted_failure` in
  `upload/uploader.rs:234-256`:

      /// Consume one attempt. Drop the entry when the budget is exhausted;
      /// otherwise requeue at the tail and persist, so the counter survives a
      /// restart (an unpersisted counter would reset and the cap would never fire).
      async fn record_failed_attempt(&mut self, mut entry: DeleteQueueEntry) {
          entry.attempts += 1;
          if entry.attempts >= self.attempts {
              Self::log_dropped(&entry);
              self.queue.persist().await;
              return;
          }
          warn!(
              "delete: attempt {} of {} failed for {}; retrying next sweep",
              entry.attempts, self.attempts, entry.job.file
          );
          self.queue.requeue(entry);
          self.queue.persist().await;
      }

- Add the two `error!` loggers, mirroring `log_dropped` / `log_terminal_drop`:

      fn log_dropped(entry: &DeleteQueueEntry) {
          error!(
              "delete: giving up on {} after {} attempts (rule {}, deployment {}, \
               digest {}); the file is left on disk and the agent will not retry it",
              entry.job.file, entry.attempts, entry.job.file_rule_id,
              entry.job.deployment_id, entry.job.digest
          );
      }

      fn log_terminal_drop(entry: &DeleteQueueEntry) {
          error!(
              "delete: giving up on {} after a permanent filesystem failure \
               (rule {}, deployment {}, digest {}, attempt {}); the file is left on \
               disk and the agent will not retry it",
              entry.job.file, entry.job.file_rule_id, entry.job.deployment_id,
              entry.job.digest, entry.attempts
          );
      }

- A successful outcome (`Deleted`, `AlreadyGone`, `Changed`) needs no new code: the entry
  is already popped and the persist drops it, attempts and all.

### M3 — tests

Three existing in-source tests change meaning, because EISDIR and ENOTDIR are now terminal:

- `deleter.rs` `mod sweep :: stat_failure_retains_entry` (ENOTDIR: job path is a child of a
  regular file) → rename to `stat_permanent_failure_drops_entry`, assert
  `deleter.queue.is_empty()`.
- `deleter.rs` `mod sweep :: hash_failure_retains_entry` (EISDIR: job path is a directory,
  so `read` fails) → `hash_permanent_failure_drops_entry`, same shape; still assert
  `target.exists()` (the directory is untouched).
- `deleter.rs` `mod sweep :: delete_failure_retains_entry` (EISDIR: `remove_file` refuses a
  directory) → `delete_permanent_failure_drops_entry`, same shape.

One existing `queue.rs` test pins the wire format and must be updated:
`mod from_snapshot :: raw_json_snapshot_loads` — wrap the job object in
`{"job": {...}, "attempts": 0}`.

New tests, all using the existing real-filesystem injection style (no fakes, no mocks):

**In `agent/src/data_uploads/retention/deleter.rs`, `mod tests`:**

Add a helper next to `temp_file`/`make_job`, since counted failures need a filesystem error
whose kind is *not* in the terminal set. A mutual symlink loop gives `ELOOP` →
`io::ErrorKind::FilesystemLoop`, which is stable, repeatable, and classified as counted:

    /// Two symlinks pointing at each other. `stat` on either fails with ELOOP,
    /// which classifies as a counted retry rather than a terminal failure.
    /// Built with `std::os::unix::fs::symlink` rather than `files::create_symlink`
    /// because the latter asserts the target exists.
    fn symlink_loop(dir: &crate::filesys::Dir) -> File {
        let a = dir.file("loop-a");
        let b = dir.file("loop-b");
        std::os::unix::fs::symlink(b.path(), a.path()).unwrap();
        std::os::unix::fs::symlink(a.path(), b.path()).unwrap();
        a
    }

Before relying on it, confirm the kind empirically once (`assert_eq!(io_kind(&err),
Some(std::io::ErrorKind::FilesystemLoop))` in the classifier test below). If the platform
maps ELOOP differently, adjust the classifier test and pick any other non-terminal kind;
the classifier unit tests below do not depend on the filesystem at all.

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
- `error_without_an_io_source_is_counted` — e.g.
  `FileSysErr::PathExistsErr(..)` → `CountedRetry`, pinning the `_ => None` arm.

  `SweepOutcome` needs `#[derive(Debug)]` for `matches!`-adjacent assertion messages; add
  it if the compiler asks.

New tests in `mod sweep`:

- `counted_failure_increments_attempts` — `dirs::temp("delete-attempts-counted")`,
  `symlink_loop(&dir)`, a `Job` whose `file` is the loop head with `ttl_secs: 0`; enqueue,
  sweep once, assert `deleter.queue.queue_entries()[0].attempts == 1` and the queue still
  has length 1; sweep again, assert `attempts == 2`.
- `attempt_cap_drops_job` — same setup but `DeleterArgs { attempts: 3, .. }`; sweep three
  times; after the third assert `deleter.queue.is_empty()` and `deleter.len() == 0`. Uses a
  small cap so the test is three sweeps, not ten.
- `default_attempts_is_ten` — `assert_eq!(DeleterArgs::default().attempts, 10);` plus a
  loop of ten sweeps over the symlink-loop job asserting the queue is non-empty for the
  first nine and empty after the tenth. This is the test that pins the headline number.
- `terminal_failure_drops_job_without_burning_attempts` — EISDIR delete case (job path is a
  directory, as in the existing `delete_failure_retains_entry`) with the default
  `attempts: 10`; one sweep; assert `deleter.queue.is_empty()`. The point is that the drop
  happens on sweep 1, not sweep 10.
- `successful_delete_clears_the_entry` — the persistence-aware sibling of the existing
  `zero_delay_entry_is_deleted_on_first_sweep`: build the deleter with a snapshot file,
  enqueue a normal temp-file job, sweep once, assert `deleter.queue.is_empty()` and
  `!tmp.file().exists()`, then drop and rebuild from the snapshot file and assert the
  rebuilt queue is empty — so no orphan attempts record survives a successful delete.

New `mod persistence` in `deleter.rs` `mod tests` (mirroring the existing
`each_drop_is_persisted_before_the_next_job` restart pattern):

- `attempts_survive_a_restart` — `dirs::temp("delete-attempts-restart")`,
  `state_path = dir.file("delete_queue.json")`, deleter built with
  `snapshot_file: Some(snapshot_file(&state_path).await)`; enqueue a symlink-loop job;
  sweep twice; `drop(deleter)`; rebuild with `SingleThreadDeleter::new(DeleterArgs {
  snapshot_file: Some(snapshot_file(&state_path).await), ..DeleterArgs::default() })`;
  assert `restored.queue.queue_entries()[0].attempts == 2`. Then sweep the *restored*
  deleter with `attempts: 3` and assert it drops — proving the budget is genuinely
  cumulative across restarts.
- `dropped_entry_is_absent_from_the_persisted_snapshot` — same setup with
  `DeleterArgs { attempts: 1, .. }`; one sweep; `drop(deleter)`; rebuild; assert
  `restored.queue.is_empty()`. This is the "entry absent from the persisted snapshot"
  requirement, checked through the real file rather than through memory.

**In `agent/src/data_uploads/retention/queue.rs`, `mod tests`:**

- `mod from_snapshot :: legacy_snapshot_shape_resets_to_empty` — write the *old* raw JSON
  (`{"entries":[{"file":"/data/a.log","size":42, ...}]}`, i.e. the string currently in
  `raw_json_snapshot_loads`) to the path, `open(&path).await`, build the queue, assert
  `queue.is_empty()`. Pins D2 as intended behavior rather than an accident.
- `mod enqueue :: new_entries_start_at_zero_attempts` — enqueue two jobs, assert
  `queue.queue_entries().iter().all(|e| e.attempts == 0)`.
- `mod requeue :: preserves_attempts` — enqueue, `pop_front`, bump `attempts` on the popped
  entry, `requeue`, assert the tail entry's `attempts`.

**In `agent/tests/data_uploads/retention/deleter.rs`** (integration, through the actor
handle — `agent/tests/data_uploads/retention/mod.rs` already declares `pub mod deleter;`,
so no registration change):

- `wedged_job_is_given_up_on_through_the_actor` — `Deleter::spawn(16, DeleterArgs {
  attempts: 2, ..DeleterArgs::default() })`; enqueue a job over a symlink loop built in a
  `dirs::temp` directory; `deleter.sweep()` twice; assert `deleter.len().await.unwrap() ==
  0`; `shutdown` + `handle.await`. This is the only new integration test — it proves the
  policy is reachable through the public `DeleterExt` surface, which is what the app wires
  up.

Lint note: any test function with 4 or more `assert_eq!` against fields of one variable
needs `// lint:allow(field-by-field-assert)` inside the body (the CI import/assert linter
flags it). Prefer fewer, more meaningful assertions.

### M4 — validation

Full preflight, push, CI. See "Validation and Acceptance".

## Concrete Steps

Working directory for **every** command below is the repo root:

    /home/ben/miru/workbench1/repos/agent-wt-delete-attempts

Confirm the starting point first:

    git rev-parse --abbrev-ref HEAD     # expect: feat/retention-delete-attempts
    git status --short                  # expect: clean

### M1

1. Edit `agent/src/data_uploads/retention/queue.rs` per M1 above.
2. Edit `agent/src/data_uploads/retention/deleter.rs` per M1 above.
3. Edit `agent/src/data_uploads/retention/mod.rs` to re-export `DeleteQueueEntry`.
4. Compile-check both the library and the tests. A plain `cargo build` skips `#[cfg(test)]`
   blocks and the `agent/tests/` targets entirely, so a rename that misses a test reference
   builds clean locally and then fails CI. Always use `--all-targets`:

       cargo check --package miru-agent --features test --all-targets

5. Run the module's tests:

       ./scripts/test.sh 2>&1 | tail -40

   Expect the retention tests to pass except the four listed in M3 as changing meaning; if
   they fail here, that is expected mid-milestone — but prefer landing M1 with them still
   green (M1 alone does not change classification, so they should still pass; only M2 flips
   them).

6. Commit:

       git add agent/src/data_uploads/retention/
       git commit -m "$(cat <<'EOF'
       feat(retention): track per-job attempt counts in the delete queue

       Adds DeleteQueueEntry { job, attempts } and changes the delete_queue.json
       snapshot shape from Vec<Job> to Vec<DeleteQueueEntry>. Pre-existing
       snapshots fail to parse and reset to an empty queue, which is fail-open:
       files are retained, never wrongly deleted.

       Co-Authored-By: Claude <noreply@anthropic.com>
       EOF
       )"

### M2

7. Edit `agent/src/data_uploads/retention/deleter.rs` per M2 above.
8. `cargo check --package miru-agent --features test --all-targets`
9. Update the four existing tests whose meaning changed (M3, first two lists) so the suite
   is green again:

       ./scripts/test.sh 2>&1 | tail -40

10. Commit:

        git add agent/src/data_uploads/retention/
        git commit -m "$(cat <<'EOF'
        feat(retention): give up on delete jobs after 10 failed attempts

        Splits SweepOutcome::Retry into CountedRetry and TerminalFailure,
        classified by io::ErrorKind. Permission-denied, read-only-filesystem,
        EISDIR, ENOTDIR and ENAMETOOLONG drop the job immediately; every other
        failure consumes one of DeleterArgs::attempts (default 10) and the
        counted-retry requeue now persists so the budget survives a restart.
        Exhaustion and terminal drops log at error! naming file, rule,
        deployment, digest and attempt count -- the file is left on disk and the
        job is never retried, so the log line is the only remaining trace.

        Co-Authored-By: Claude <noreply@anthropic.com>
        EOF
        )"

### M3

11. Add the new tests per M3.
12. Run:

        ./scripts/test.sh

13. Re-ratchet coverage. Do **not** hand-edit `.covgate` values:

        ./scripts/covgate.sh
        ./scripts/update-covgates.sh   # only if covgate.sh reports a module below its gate
        git diff -- '**/.covgate'

    Expect `agent/src/data_uploads/retention/.covgate` to move at most a little from
    `98.39`. If `agent/src/workers/.covgate` shows a local-only failure, leave it alone —
    see the note in Validation.

14. Commit:

        git add agent/src/data_uploads/retention/ agent/tests/data_uploads/retention/ '**/.covgate'
        git commit -m "$(cat <<'EOF'
        test(retention): cover delete attempt accounting and give-up behavior

        Classifier unit tests over constructed FileSysErr values, sweep-level
        tests using a real ELOOP symlink loop for counted failures and EISDIR /
        ENOTDIR for terminal ones, restart tests proving the attempt count is
        cumulative across a rebuild from delete_queue.json, and one integration
        test through the Deleter actor handle.

        Co-Authored-By: Claude <noreply@anthropic.com>
        EOF
        )"

### M4

15. Refresh the lockfile and run the full pre-push gate:

        ./scripts/update-deps.sh
        ./scripts/preflight.sh

    `preflight.sh` runs lint, covgate, tools lint and tools tests in parallel. `lint.sh`
    auto-fixes some findings (`cargo fmt`, `clippy --fix`), so re-check afterwards:

        git status --short

    If it made changes, amend them into the M3 commit (or add a fixup commit — either is
    fine, the branch is not yet pushed):

        git add -A && git commit --amend --no-edit

16. Push and open a draft PR:

        git push -u origin feat/retention-delete-attempts

17. Watch CI to green, then take the PR out of draft. Per the repo's tooling notes, `gh`'s
    GraphQL-backed commands are unreliable here — use the REST API:

        gh api repos/mirurobotics/agent/commits/$(git rev-parse HEAD)/check-runs \
          --jq '.check_runs[] | "\(.name): \(.conclusion // .status)"'

18. If the PR body or state needs editing, use `gh api` REST rather than `gh pr edit`.

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

1. **A counted failure consumes exactly one attempt per sweep.** `./scripts/test.sh` runs
   `counted_failure_increments_attempts`: one sweep over a wedged (ELOOP) delete target
   leaves `attempts == 1` and the entry still queued; a second leaves `attempts == 2`.
2. **Ten attempts is the give-up point.** `default_attempts_is_ten` asserts
   `DeleterArgs::default().attempts == 10` and that the queue is non-empty after nine
   sweeps and empty after the tenth.
3. **Exhaustion removes the entry from disk, not just from memory.**
   `dropped_entry_is_absent_from_the_persisted_snapshot` drops the deleter after the final
   sweep and rebuilds from `delete_queue.json`; the rebuilt queue is empty.
4. **A terminal failure does not burn the budget.**
   `terminal_failure_drops_job_without_burning_attempts` drops an EISDIR target on the
   *first* sweep even with the default budget of 10.
5. **The attempt count is cumulative across restarts.** `attempts_survive_a_restart` sweeps
   twice, drops the deleter, rebuilds from the snapshot file, and finds `attempts == 2`;
   the restored deleter then reaches its cap.
6. **A successful delete clears everything.** `successful_delete_clears_the_entry` leaves an
   empty queue, an unlinked file, and an empty persisted snapshot.
7. **The actor surface honors the policy.** `wedged_job_is_given_up_on_through_the_actor`
   drives it entirely through `DeleterExt::enqueue`/`sweep`/`len`, the same API
   `agent/src/app/state.rs` wires up.
8. **The give-up is loud.** Run the agent (or grep the sources) and confirm the drop paths
   are `error!`, name the file path, `file_rule_id`, `deployment_id`, `digest` and the
   attempt count, and say the file is left on disk. A `warn!` here would be a defect: the
   log line is the only surviving record of a leaked file.

### What acceptance does *not* claim

The dropped file is not deleted and is not rediscovered. That is the accepted trade (D7).
If the file is later modified, the scanner re-emits it and it gets a fresh budget — which
is correct, not a loop.

## Idempotence and Recovery

- Every step is a source edit plus a rerun of a script; all are safe to repeat.
  `./scripts/test.sh`, `./scripts/covgate.sh`, `./scripts/lint.sh` and
  `./scripts/preflight.sh` are read-only with respect to your intent — `lint.sh` mutates
  source only by auto-formatting and applying clippy fixes, which is idempotent; always
  re-run `git status --short` after it.
- `./scripts/update-covgates.sh` rewrites `.covgate` files in place. Re-running is safe.
  If it ratchets a module you did not touch, `git checkout -- <path>/.covgate` that file.
- **Riskiest step is M1's snapshot shape change**, and it is only risky on a machine with a
  real agent root. It is not destructive in the repo: no test fixture on disk is shared
  across tests (each uses its own `dirs::temp`). If you want to prove the reset behavior
  on a live agent root, back the file up first:

      cp ~/.miru/delete_queue.json /tmp/delete_queue.json.bak

- To abandon the work at any point before pushing:

      git reset --hard 95bc2a5

  After pushing, prefer a revert commit over a force-push.
- If `./scripts/test.sh` fails only in `agent/src/data_uploads/retention/` after M2, that is
  the expected mid-milestone state for the three renamed EISDIR/ENOTDIR tests; finish step 9
  before concluding anything is wrong.

## Outcomes & Retrospective

_(filled at completion)_
