// standard crates
use std::collections::{HashMap, HashSet};
use std::fs::Metadata;
use std::time::{Duration, Instant, SystemTime};

// internal crates
use crate::clock::Clock;
use crate::cooldown;
use crate::data_uploads::retention::{
    errors::*,
    job::{add_duration, Expiry, Job},
    queue::{DeleteQueueSnapshotFile, Queue, QueueEntry},
};
use crate::filesys::{errors::FileSysErr, files};
use crate::queue::Persistence;

// external crates
use chrono::{DateTime, TimeDelta, Utc};
use tracing::{error, info, warn};
use uuid::Uuid;

const DEFAULT_QUEUE_CAPACITY: usize = 4096;
const DEFAULT_ATTEMPTS: u32 = 10;
const DEFAULT_CHECKPOINT_INTERVAL: Duration = Duration::from_secs(10 * 60);

pub struct DeleterArgs {
    /// Server time decides when a TTL runs out, and the monotonic clock
    /// counts it when server time is not known. The system clock only sets
    /// retry backoff deadlines.
    pub clock: Clock,
    pub queue_capacity: usize,
    pub snapshot_file: Option<DeleteQueueSnapshotFile>,
    pub attempts: u32,
    pub backoff: cooldown::Backoff,
    /// How often each job's remaining TTL is saved. A restart that has not
    /// heard from the server yet resumes counting from the latest save.
    pub checkpoint_interval: Duration,
}

impl Default for DeleterArgs {
    fn default() -> Self {
        Self {
            clock: Clock::new(),
            queue_capacity: DEFAULT_QUEUE_CAPACITY,
            snapshot_file: None,
            attempts: DEFAULT_ATTEMPTS,
            backoff: cooldown::Backoff {
                base_secs: 10,
                growth_factor: 2,
                max_secs: 3600,
            },
            checkpoint_interval: DEFAULT_CHECKPOINT_INTERVAL,
        }
    }
}

/// A job's TTL counted on the monotonic clock in this process: `remaining`
/// was left at `started`.
#[derive(Clone, Copy, Debug)]
struct Countdown {
    started: Instant,
    remaining: Duration,
}

impl Countdown {
    fn left(&self, now: Instant) -> Duration {
        self.remaining
            .saturating_sub(now.saturating_duration_since(self.started))
    }
}

pub struct DeleterImpl {
    queue: Queue,
    clock: Clock,
    attempts: u32,
    backoff: cooldown::Backoff,
    checkpoint_interval: Duration,
    /// In-memory TTL countdowns by queue entry id. Monotonic instants mean
    /// nothing after a restart, so these are never saved.
    countdowns: HashMap<Uuid, Countdown>,
    /// When the snapshot was loaded: the countdown of a loaded job resumes
    /// from its saved remaining TTL at this instant.
    loaded_at: Instant,
    last_checkpoint: Option<Instant>,
}

/// Outcome of considering one queued job during a sweep.
enum SweepOutcome {
    /// The sweep could not resolve the job; consumes one attempt.
    Failed,
    /// File was deleted; drop the job.
    Deleted,
    /// File is already gone; drop the job.
    AlreadyGone,
    /// On-disk identity no longer matches the tagged file; drop without deleting.
    Changed,
}

/// What [`DeleterImpl::refresh_expiries`] found.
#[derive(Default)]
struct Refresh {
    due: HashSet<Uuid>,
    countdowns: HashMap<Uuid, Countdown>,
    given_server_deadline: usize,
}

impl DeleterImpl {
    pub fn new(args: DeleterArgs) -> Self {
        let queue = match args.snapshot_file {
            Some(snapshot_file) => {
                Queue::from_snapshot(args.queue_capacity, snapshot_file, Persistence::BestEffort)
            }
            None => Queue::new(args.queue_capacity),
        };
        let loaded_at = args.clock.monotonic_now();
        Self {
            queue,
            clock: args.clock,
            attempts: args.attempts,
            backoff: args.backoff,
            checkpoint_interval: args.checkpoint_interval,
            countdowns: HashMap::new(),
            loaded_at,
            last_checkpoint: None,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.queue.len()
    }

    /// Queue `job` with its TTL starting now. Any expiry the producer set is
    /// replaced: the TTL is counted in server time when the agent knows it,
    /// and on the monotonic clock otherwise, never on the system clock.
    pub(crate) async fn enqueue(&mut self, mut job: Job) -> Result<(), DeleteErr> {
        job.expiry = Expiry::start(job.ttl_secs, self.clock.server_now());
        let countdown = Countdown {
            started: self.clock.monotonic_now(),
            remaining: job.ttl(),
        };
        let id = self.queue.enqueue(job).await?;
        self.countdowns.insert(id, countdown);
        Ok(())
    }

    pub(crate) async fn sweep(&mut self) -> Result<(), DeleteErr> {
        let due = self.refresh_expiries().await;
        let is_due = |entry: &QueueEntry| due.contains(&entry.id);
        let now = self.clock.system_now();
        // Budget: one visit per entry that is due at `now`. A retried entry is
        // requeued at the tail, behind every entry not yet visited, so this
        // budget is exactly enough to visit each due entry once and never
        // twice.
        for _ in 0..self.queue.count_ready_where(now, is_due) {
            let Some(entry) = self.queue.next_ready_where(now, is_due) else {
                break;
            };
            match Self::sweep_entry(&entry.job).await {
                SweepOutcome::Failed => self.handle_counted_failure(entry, now).await,
                SweepOutcome::Deleted | SweepOutcome::AlreadyGone | SweepOutcome::Changed => {
                    self.queue.ack(entry.id).await;
                }
            }
        }
        Ok(())
    }

    /// Work out how much TTL each queued job has left, and return the ids of
    /// the jobs whose TTL has run out.
    ///
    /// A job with a server deadline is measured against server time when the
    /// agent knows it. Otherwise its countdown carries on from this process's
    /// monotonic count, or, for a job loaded from the snapshot, from the TTL
    /// saved at the latest checkpoint, counted from when it was loaded. Once
    /// server time is known, a job without a server deadline is given one, so
    /// its TTL keeps its calendar meaning across restarts. Every
    /// `checkpoint_interval` the TTL left is saved.
    async fn refresh_expiries(&mut self) -> HashSet<Uuid> {
        let server_now = self.clock.server_now();
        let monotonic_now = self.clock.monotonic_now();
        let checkpoint = self.last_checkpoint.is_none_or(|at| {
            monotonic_now.saturating_duration_since(at) >= self.checkpoint_interval
        });
        if checkpoint {
            self.last_checkpoint = Some(monotonic_now);
        }

        let previous = &self.countdowns;
        let loaded_at = self.loaded_at;
        let mut refresh = Refresh::default();
        self.queue
            .update_jobs(|id, job| {
                let left = match (job.expiry.server_expires_at, server_now) {
                    (Some(deadline), Some(now)) => {
                        (deadline - now).to_std().unwrap_or(Duration::ZERO)
                    }
                    _ => previous
                        .get(&id)
                        .copied()
                        .unwrap_or(Countdown {
                            started: loaded_at,
                            remaining: job.expiry.saved_remaining(job.ttl()),
                        })
                        .left(monotonic_now),
                };
                refresh.countdowns.insert(
                    id,
                    Countdown {
                        started: monotonic_now,
                        remaining: left,
                    },
                );
                if left.is_zero() {
                    refresh.due.insert(id);
                }
                Self::update_expiry(&mut job.expiry, left, server_now, checkpoint, &mut refresh)
            })
            .await;

        self.countdowns = refresh.countdowns;
        if refresh.given_server_deadline > 0 {
            info!(
                "delete: {} job(s) now count their TTL in server time",
                refresh.given_server_deadline
            );
        }
        refresh.due
    }

    /// Record `left` on `expiry`: as a server deadline if it has none and
    /// server time is known, and as the saved remaining TTL at a checkpoint.
    /// Returns whether anything changed.
    fn update_expiry(
        expiry: &mut Expiry,
        left: Duration,
        server_now: Option<DateTime<Utc>>,
        checkpoint: bool,
        refresh: &mut Refresh,
    ) -> bool {
        let mut changed = false;
        if let (None, Some(now)) = (expiry.server_expires_at, server_now) {
            expiry.server_expires_at = Some(add_duration(now, left));
            refresh.given_server_deadline += 1;
            changed = true;
        }
        if checkpoint {
            // round up, so a restart never shortens the TTL
            let secs = left.as_secs() + u64::from(left.subsec_nanos() > 0);
            if expiry.remaining_ttl_secs != Some(secs) {
                expiry.remaining_ttl_secs = Some(secs);
                changed = true;
            }
        }
        changed
    }

    /// Counted failures bump the attempt budget. Drop when the budget is
    /// exhausted; otherwise stamp the growing backoff and requeue at the tail.
    async fn handle_counted_failure(&mut self, mut entry: QueueEntry, now: DateTime<Utc>) {
        entry.attempts += 1;
        if entry.attempts >= self.attempts {
            Self::log_exhausted_drop(&entry);
            self.queue.ack(entry.id).await;
            return;
        }

        let wait = cooldown::calc(&self.backoff, entry.attempts - 1).max(0);
        warn!(
            "delete: attempt {} of {} failed for {}; retrying in {wait}s",
            entry.attempts, self.attempts, entry.job.file
        );
        let next_attempt_at = now + TimeDelta::seconds(wait);
        self.queue
            .retry(entry.id, entry.attempts, next_attempt_at)
            .await;
    }

    /// Consider one selected job: delete only when the on-disk file still
    /// matches the tagged size/mtime/digest. Readiness is the queue's concern
    /// — a job that is not due is never selected.
    async fn sweep_entry(entry: &Job) -> SweepOutcome {
        let metadata = match Self::stat_file(entry).await {
            Ok(metadata) => metadata,
            Err(outcome) => return outcome,
        };
        if let Some(outcome) = Self::check_file_identity(entry, &metadata).await {
            return outcome;
        }
        Self::delete_file(entry).await
    }

    async fn stat_file(entry: &Job) -> Result<Metadata, SweepOutcome> {
        match files::metadata(&entry.file).await {
            Ok(metadata) => Ok(metadata),
            Err(FileSysErr::PathDoesNotExistErr(_)) => {
                info!("delete: {} already gone; dropping entry", entry.file);
                Err(SweepOutcome::AlreadyGone)
            }
            Err(err) => {
                warn!("delete: failed to stat {}: {err:?}", entry.file);
                Err(SweepOutcome::Failed)
            }
        }
    }

    async fn check_file_identity(entry: &Job, metadata: &Metadata) -> Option<SweepOutcome> {
        if metadata.len() != entry.size {
            info!(
                "delete: {} changed since upload; dropping without deleting",
                entry.file
            );
            return Some(SweepOutcome::Changed);
        }
        let mtime = DateTime::<Utc>::from(metadata.modified().unwrap_or(SystemTime::now()));
        if mtime == entry.mtime {
            return None;
        }
        Self::check_digest_mismatch(entry).await
    }

    async fn check_digest_mismatch(entry: &Job) -> Option<SweepOutcome> {
        match files::hash(&entry.file).await {
            Ok(digest) if digest == entry.digest => None,
            Ok(_) => {
                info!(
                    "delete: {} changed since upload; dropping without deleting",
                    entry.file
                );
                Some(SweepOutcome::Changed)
            }
            Err(FileSysErr::PathDoesNotExistErr(_)) => {
                info!("delete: {} already gone; dropping entry", entry.file);
                Some(SweepOutcome::AlreadyGone)
            }
            Err(err) => {
                warn!("delete: failed to hash {}: {err:?}", entry.file);
                Some(SweepOutcome::Failed)
            }
        }
    }

    async fn delete_file(entry: &Job) -> SweepOutcome {
        match files::delete(&entry.file).await {
            Ok(()) => {
                info!(
                    "delete: deleted {} (rule {}, deployment {})",
                    entry.file, entry.file_rule_id, entry.deployment_id
                );
                SweepOutcome::Deleted
            }
            Err(err) => {
                warn!("delete: failed to delete {}: {err:?}", entry.file);
                SweepOutcome::Failed
            }
        }
    }

    fn log_exhausted_drop(entry: &QueueEntry) {
        error!(
            "delete: giving up on {} after {} attempts (rule {}, deployment {}, \
             digest {}); the file is left on disk and the agent will not retry it",
            entry.job.file,
            entry.attempts,
            entry.job.file_rule_id,
            entry.job.deployment_id,
            entry.job.digest
        );
    }
}

#[cfg(test)]
mod tests {
    // internal crates
    use super::{DeleterArgs, DeleterImpl, SweepOutcome};
    use crate::cooldown;
    use crate::data_uploads::retention::job::{Expiry, Job};
    use crate::data_uploads::retention::queue::{DeleteQueueSnapshot, DeleteQueueSnapshotFile};
    use crate::filesys::{dirs, files, Dir, File, PathExt, WriteOptions};
    use crate::test_utils::{
        clock::FakeClock as Clock, filesys::dirs as test_dirs, filesys::files as test_files,
        retention::undeletable_dir_job,
    };

    // external crates
    use chrono::{DateTime, Utc};

    // =============================== TEST HELPERS ================================= //

    /// A real on-disk temp file holding `contents`; the returned guard deletes
    /// it on drop.
    async fn temp_file(contents: &[u8]) -> test_files::TempFile {
        let tmp = test_files::temp("delete-sweep-test").unwrap();
        files::write_bytes(tmp.file(), contents, WriteOptions::OVERWRITE_NONATOMIC)
            .await
            .unwrap();
        tmp
    }

    /// A `Job` for `file` whose size/mtime/digest reflect the file's
    /// current on-disk state.
    async fn make_job(file: &File, observed_secs: i64, ttl_secs: u64) -> Job {
        let observed_at = DateTime::from_timestamp(observed_secs, 0).unwrap();
        Job {
            file: file.clone(),
            size: files::size(file).await.unwrap(),
            digest: files::hash(file).await.unwrap(),
            mtime: DateTime::<Utc>::from(files::last_modified(file).await.unwrap()),
            first_observed_at: observed_at,
            last_observed_at: observed_at,
            ttl_secs,
            file_rule_id: "file_rule_1".to_string(),
            deployment_id: "dpl_1".to_string(),
            expiry: Expiry::default(),
        }
    }

    /// `job` as the deleter queues it before server time is known: the
    /// whole TTL left and no server deadline.
    fn stamped(job: Job) -> Job {
        Job {
            expiry: Expiry::start(job.ttl_secs, None),
            ..job
        }
    }

    /// A zero-length backoff: a counted failure is retryable immediately, so a
    /// test can drive successive attempts without stepping the clock. The
    /// backoff's own growth is pinned separately, in `mod backoff`.
    const NO_BACKOFF: cooldown::Backoff = cooldown::Backoff {
        base_secs: 0,
        growth_factor: 2,
        max_secs: 0,
    };

    /// A deleter with the injected clock, default capacity, no persistence,
    /// and no backoff.
    fn deleter(clock: &Clock) -> DeleterImpl {
        DeleterImpl::new(DeleterArgs {
            clock: clock.clock(),
            backoff: NO_BACKOFF,
            ..DeleterArgs::default()
        })
    }

    /// A persistence handle for the snapshot at `file`.
    async fn snapshot_file(file: &File) -> DeleteQueueSnapshotFile {
        DeleteQueueSnapshotFile::new_with_default(file.clone(), DeleteQueueSnapshot::default())
            .await
            .unwrap()
    }

    /// Epoch seconds as a `DateTime<Utc>`.
    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).unwrap()
    }

    /// A `Job` for a path whose stat cannot succeed. The recorded
    /// size/digest/mtime are never compared: the sweep fails at the stat step
    /// before any identity check runs. `make_job` is unusable here — its
    /// `files::size`/`files::hash` calls would fail in setup.
    fn wedged_job(file: File) -> Job {
        let at = DateTime::from_timestamp(1000, 0).unwrap();
        Job {
            file,
            size: 0,
            digest: "sha256:unused".to_string(),
            mtime: DateTime::from_timestamp(1000, 0).unwrap(),
            first_observed_at: at,
            last_observed_at: at,
            ttl_secs: 0,
            file_rule_id: "rule_1".to_string(),
            deployment_id: "dpl_1".to_string(),
            expiry: Expiry::default(),
        }
    }

    mod enqueue {
        use super::*;
        use crate::data_uploads::retention::errors::{DeleteErr, EnqueueErr};

        #[tokio::test]
        async fn appends_the_job() {
            let tmp = temp_file(b"aaaa").await;
            let clock = Clock::new(1000);
            let mut deleter = deleter(&clock);
            let job = make_job(tmp.file(), 1000, 0).await;

            deleter.enqueue(job.clone()).await.unwrap();

            assert_eq!(deleter.queue.entries(), [stamped(job)]);
        }

        #[tokio::test]
        async fn persists_to_disk() {
            let dir = test_dirs::temp("delete-enqueue-persist").unwrap();
            let state_path = dir.file("delete_queue.json");
            let tmp = temp_file(b"aaaa").await;
            let clock = Clock::new(1000);
            let mut deleter = DeleterImpl::new(DeleterArgs {
                clock: clock.clock(),
                snapshot_file: Some(snapshot_file(&state_path).await),
                ..DeleterArgs::default()
            });
            let job = make_job(tmp.file(), 1000, 500).await;
            deleter.enqueue(job.clone()).await.unwrap();
            drop(deleter);

            let restored = DeleterImpl::new(DeleterArgs {
                snapshot_file: Some(snapshot_file(&state_path).await),
                ..DeleterArgs::default()
            });
            assert_eq!(restored.queue.entries(), [stamped(job)]);
        }

        #[tokio::test]
        async fn full_queue_returns_queue_full_err() {
            let tmp_a = temp_file(b"aaaa").await;
            let tmp_b = temp_file(b"bbbb").await;
            let clock = Clock::new(1000);
            let mut deleter = DeleterImpl::new(DeleterArgs {
                clock: clock.clock(),
                queue_capacity: 1,
                ..DeleterArgs::default()
            });
            let first = make_job(tmp_a.file(), 1000, 0).await;
            deleter.enqueue(first.clone()).await.unwrap();

            let err = deleter
                .enqueue(make_job(tmp_b.file(), 1000, 0).await)
                .await
                .unwrap_err();
            let DeleteErr::EnqueueErr(EnqueueErr::QueueFullErr(full)) = err else {
                panic!("expected QueueFullErr, got: {err:?}");
            };
            assert_eq!(full.capacity, 1);
            assert_eq!(full.name, tmp_b.file().to_string());
        }

        #[tokio::test]
        async fn persist_failure_is_swallowed() {
            let dir = test_dirs::temp("delete-enqueue-persist-fail").unwrap();
            let state_path = dir.file("delete_queue.json");
            let tmp = temp_file(b"aaaa").await;
            let clock = Clock::new(1000);
            let mut deleter = DeleterImpl::new(DeleterArgs {
                clock: clock.clock(),
                snapshot_file: Some(snapshot_file(&state_path).await),
                ..DeleterArgs::default()
            });

            // make the snapshot path unwritable: a DIRECTORY now sits there.
            files::delete(&state_path).await.unwrap();
            dirs::create(&Dir::new(state_path.path().clone()))
                .await
                .unwrap();

            let job = make_job(tmp.file(), 1000, 0).await;
            deleter.enqueue(job.clone()).await.unwrap();
            assert_eq!(deleter.queue.entries(), [stamped(job)]);
        }
    }

    mod sweep {
        use super::*;

        // Pins the walk's exactly-once guarantee: a not-due entry at the head
        // is requeued at the tail and must not stop the pass — a due job
        // behind it still deletes in the same sweep.
        #[tokio::test]
        async fn due_entry_behind_a_not_due_entry_is_still_swept() {
            let waiting = temp_file(b"aaaa").await;
            let due = temp_file(b"bbbb").await;
            let clock = Clock::new(1000);
            let mut deleter = deleter(&clock);
            let waiting_job = make_job(waiting.file(), 1000, 500).await;
            deleter.enqueue(waiting_job.clone()).await.unwrap();
            deleter
                .enqueue(make_job(due.file(), 1000, 0).await)
                .await
                .unwrap();

            deleter.sweep().await.unwrap();

            assert!(!due.file().exists());
            assert!(waiting.file().exists());
            assert_eq!(deleter.queue.entries(), [stamped(waiting_job)]);
        }

        // Same-path duplicates coexist in the queue (no dedup on enqueue);
        // the safety argument is that a sweep resolves them: the first job
        // deletes the file and the stale duplicate drops as already-gone in
        // the same pass.
        #[tokio::test]
        async fn same_path_duplicate_resolves_in_one_sweep() {
            let tmp = temp_file(b"aaaa").await;
            let clock = Clock::new(1000);
            let mut deleter = deleter(&clock);
            deleter
                .enqueue(make_job(tmp.file(), 900, 0).await)
                .await
                .unwrap();
            deleter
                .enqueue(make_job(tmp.file(), 1000, 0).await)
                .await
                .unwrap();

            deleter.sweep().await.unwrap();

            assert!(deleter.queue.is_empty());
            assert!(!tmp.file().exists());
        }

        #[tokio::test]
        async fn positive_ttl_entry_waits_for_its_ttl() {
            let tmp = temp_file(b"aaaa").await;
            let clock = Clock::new(1000);
            let mut deleter = deleter(&clock);
            let record = make_job(tmp.file(), 1000, 500).await;
            deleter.enqueue(record.clone()).await.unwrap();

            // not yet due: 0s and 499s into the 500s TTL.
            deleter.sweep().await.unwrap();
            clock.advance(499);
            deleter.sweep().await.unwrap();
            assert_eq!(deleter.queue.entries(), [stamped(record)]);
            assert!(tmp.file().exists());

            // due exactly when the TTL runs out.
            clock.advance(1);
            deleter.sweep().await.unwrap();
            assert!(deleter.queue.is_empty());
            assert!(!tmp.file().exists());
        }

        #[tokio::test]
        async fn missing_file_is_dropped_as_success() {
            let tmp = temp_file(b"aaaa").await;
            let clock = Clock::new(1000);
            let mut deleter = deleter(&clock);
            deleter
                .enqueue(make_job(tmp.file(), 1000, 0).await)
                .await
                .unwrap();
            files::delete(tmp.file()).await.unwrap();

            deleter.sweep().await.unwrap();

            assert!(deleter.queue.is_empty());
        }

        // An embedded NUL makes metadata fail with InvalidInput on every
        // platform, rather than a platform-dependent flavor of NotFound.
        // The failure is counted like any other, not specially classified.
        #[tokio::test]
        async fn stat_failure_counts_an_attempt() {
            let invalid = File::new("invalid\0path");
            let clock = Clock::new(1000);
            let mut deleter = deleter(&clock);
            deleter.enqueue(wedged_job(invalid)).await.unwrap();

            deleter.sweep().await.unwrap();

            assert_eq!(deleter.queue.queue_entries()[0].attempts, 1);
        }

        // ENOTDIR: the recorded path's parent is a file. Unix reports
        // NotADirectory, which is counted like any other stat failure.
        #[cfg(unix)]
        #[tokio::test]
        async fn parent_is_a_file_counts_an_attempt() {
            let parent = temp_file(b"not a dir").await;
            let child = File::new(parent.file().path().join("child"));
            let clock = Clock::new(1000);
            let mut deleter = deleter(&clock);
            deleter.enqueue(wedged_job(child)).await.unwrap();

            deleter.sweep().await.unwrap();

            assert_eq!(deleter.queue.queue_entries()[0].attempts, 1);
        }

        // Windows reports NotFound for a path beneath a file, so the entry is
        // dropped as already gone.
        #[cfg(windows)]
        #[tokio::test]
        async fn parent_is_a_file_is_treated_as_already_gone() {
            let parent = temp_file(b"not a dir").await;
            let child = File::new(parent.file().path().join("child"));
            let clock = Clock::new(1000);
            let mut deleter = deleter(&clock);
            deleter.enqueue(wedged_job(child)).await.unwrap();

            deleter.sweep().await.unwrap();

            assert!(deleter.queue.is_empty());
        }

        #[tokio::test]
        async fn size_changed_file_is_dropped_without_deleting() {
            let tmp = temp_file(b"aaaa").await;
            let clock = Clock::new(1000);
            let mut deleter = deleter(&clock);
            deleter
                .enqueue(make_job(tmp.file(), 1000, 0).await)
                .await
                .unwrap();

            // the file grows after the record was taken.
            files::write_bytes(tmp.file(), b"aaaa-grown", WriteOptions::OVERWRITE_NONATOMIC)
                .await
                .unwrap();

            deleter.sweep().await.unwrap();

            assert!(deleter.queue.is_empty());
            assert!(tmp.file().exists());
        }

        #[tokio::test]
        async fn mtime_changed_content_unchanged_is_deleted() {
            let tmp = temp_file(b"aaaa").await;
            let clock = Clock::new(1000);
            let mut deleter = deleter(&clock);
            let mut record = make_job(tmp.file(), 1000, 0).await;
            // same size, different recorded mtime: the re-stat mismatches, but
            // the untouched file's digest still matches, so the sweep deletes.
            record.mtime += chrono::Duration::seconds(1);
            deleter.enqueue(record).await.unwrap();

            deleter.sweep().await.unwrap();

            assert!(deleter.queue.is_empty());
            assert!(!tmp.file().exists());
        }

        #[tokio::test]
        async fn mtime_and_content_changed_is_dropped_without_deleting() {
            let tmp = temp_file(b"aaaa").await;
            let clock = Clock::new(1000);
            let mut deleter = deleter(&clock);
            let mut record = make_job(tmp.file(), 1000, 0).await;
            // deterministic mtime mismatch: the record carries a sentinel.
            record.mtime = DateTime::from_timestamp(1, 0).unwrap();
            deleter.enqueue(record).await.unwrap();

            // same size, different content: the size check passes at 4 bytes
            // and the digest branch drops the entry without deleting.
            files::write_bytes(tmp.file(), b"bbbb", WriteOptions::OVERWRITE_NONATOMIC)
                .await
                .unwrap();

            deleter.sweep().await.unwrap();

            assert!(deleter.queue.is_empty());
            assert!(tmp.file().exists());
        }

        // EISDIR: the recorded path is a directory, so the hash read fails;
        // the pass must count an attempt and keep going, not panic.
        #[tokio::test]
        async fn hash_failure_counts_an_attempt() {
            let dir = test_dirs::temp("delete-hash-eisdir").unwrap();
            let clock = Clock::new(1000);
            let mut deleter = deleter(&clock);
            // sentinel mtime: the re-stat mismatches, forcing the re-hash.
            let job = Job {
                mtime: at(1),
                ..undeletable_dir_job(&dir, at(1000)).await
            };
            let target = job.file.clone();
            deleter.enqueue(job).await.unwrap();

            deleter.sweep().await.unwrap();

            assert_eq!(deleter.queue.queue_entries()[0].attempts, 1);
            assert!(target.exists());
        }

        #[tokio::test]
        async fn zero_delay_entry_is_deleted_on_first_sweep() {
            let tmp = temp_file(b"aaaa").await;
            let clock = Clock::new(1000);
            let mut deleter = deleter(&clock);
            deleter
                .enqueue(make_job(tmp.file(), 1000, 0).await)
                .await
                .unwrap();

            deleter.sweep().await.unwrap();

            assert!(deleter.queue.is_empty());
            assert!(!tmp.file().exists());
        }

        // EISDIR: the recorded path is a directory and unlink refuses
        // directories, so files::delete fails at the unlink step.
        #[tokio::test]
        async fn unlink_failure_counts_an_attempt() {
            let dir = test_dirs::temp("delete-eisdir").unwrap();
            let clock = Clock::new(1000);
            let mut deleter = deleter(&clock);
            let job = undeletable_dir_job(&dir, at(1000)).await;
            let target = job.file.clone();
            deleter.enqueue(job).await.unwrap();

            deleter.sweep().await.unwrap();

            assert_eq!(deleter.queue.queue_entries()[0].attempts, 1);
            assert!(target.exists());
        }

        #[tokio::test]
        async fn each_drop_is_persisted_before_the_next_job() {
            let dir = test_dirs::temp("delete-sweep-persist").unwrap();
            let state_path = dir.file("delete_queue.json");
            let due = temp_file(b"aaaa").await;
            let waiting = temp_file(b"bbbb").await;
            let clock = Clock::new(1000);
            let mut deleter = DeleterImpl::new(DeleterArgs {
                clock: clock.clock(),
                snapshot_file: Some(snapshot_file(&state_path).await),
                ..DeleterArgs::default()
            });
            let waiting_job = make_job(waiting.file(), 1000, 500).await;
            deleter
                .enqueue(make_job(due.file(), 1000, 0).await)
                .await
                .unwrap();
            deleter.enqueue(waiting_job.clone()).await.unwrap();

            deleter.sweep().await.unwrap();
            drop(deleter);

            // the due job was acked-and-persisted (FileQueue::ack)
            // before the waiting job was considered, so a rebuild sees only
            // the waiting job.
            let restored = DeleterImpl::new(DeleterArgs {
                snapshot_file: Some(snapshot_file(&state_path).await),
                ..DeleterArgs::default()
            });
            assert!(!due.file().exists());
            assert!(waiting.file().exists());
            assert_eq!(restored.queue.entries(), [stamped(waiting_job)]);
        }

        // A not-due entry is never selected, so a sweep leaves it exactly
        // where it is — same position, same id — even while a due entry
        // behind it resolves and rewrites the snapshot.
        #[tokio::test]
        async fn not_due_entries_are_left_untouched() {
            let dir = test_dirs::temp("delete-sweep-not-due").unwrap();
            let state_path = dir.file("delete_queue.json");
            let waiting_a = temp_file(b"aaaa").await;
            let waiting_b = temp_file(b"bbbb").await;
            let due = temp_file(b"cccc").await;
            let clock = Clock::new(1000);
            let mut deleter = DeleterImpl::new(DeleterArgs {
                clock: clock.clock(),
                snapshot_file: Some(snapshot_file(&state_path).await),
                ..DeleterArgs::default()
            });
            for job in [
                make_job(waiting_a.file(), 1000, 500).await,
                make_job(waiting_b.file(), 1000, 900).await,
                make_job(due.file(), 1000, 0).await,
            ] {
                deleter.enqueue(job).await.unwrap();
            }
            let before = snapshot_file(&state_path).await.read().entries.clone();

            deleter.sweep().await.unwrap();

            let after = snapshot_file(&state_path).await.read().entries.clone();
            assert_eq!(after, before[..2]);
            assert!(waiting_a.file().exists());
            assert!(waiting_b.file().exists());
            assert!(!due.file().exists());
        }

        #[tokio::test]
        async fn vanished_file_at_the_hash_step_is_already_gone() {
            let job = wedged_job(File::new("/nonexistent/miru-delete-test/a.log"));

            let outcome = DeleterImpl::check_digest_mismatch(&job).await;

            assert!(matches!(outcome, Some(SweepOutcome::AlreadyGone)));
        }

        #[tokio::test]
        async fn counted_failure_increments_attempts() {
            let dir = test_dirs::temp("delete-attempts-counted").unwrap();
            let clock = Clock::new(1000);
            let mut deleter = deleter(&clock);
            deleter
                .enqueue(undeletable_dir_job(&dir, at(1000)).await)
                .await
                .unwrap();

            deleter.sweep().await.unwrap();
            assert_eq!(deleter.queue.queue_entries()[0].attempts, 1);
            assert_eq!(deleter.queue.len(), 1);

            deleter.sweep().await.unwrap();
            assert_eq!(deleter.queue.queue_entries()[0].attempts, 2);
        }

        #[tokio::test]
        async fn attempt_cap_drops_job() {
            let dir = test_dirs::temp("delete-attempts-cap").unwrap();
            let clock = Clock::new(1000);
            let mut deleter = DeleterImpl::new(DeleterArgs {
                clock: clock.clock(),
                attempts: 3,
                backoff: NO_BACKOFF,
                ..DeleterArgs::default()
            });
            deleter
                .enqueue(undeletable_dir_job(&dir, at(1000)).await)
                .await
                .unwrap();

            deleter.sweep().await.unwrap();
            assert!(!deleter.queue.is_empty());
            deleter.sweep().await.unwrap();
            assert!(!deleter.queue.is_empty());

            deleter.sweep().await.unwrap();
            assert!(deleter.queue.is_empty());
        }

        #[tokio::test]
        async fn default_attempts_is_ten() {
            assert_eq!(DeleterArgs::default().attempts, 10);

            let dir = test_dirs::temp("delete-attempts-default").unwrap();
            let clock = Clock::new(1000);
            let mut deleter = deleter(&clock);
            deleter
                .enqueue(undeletable_dir_job(&dir, at(1000)).await)
                .await
                .unwrap();

            for i in 1..=10 {
                deleter.sweep().await.unwrap();
                assert_eq!(deleter.queue.is_empty(), i == 10, "after sweep {i}");
            }
        }

        #[tokio::test]
        async fn successful_delete_clears_the_entry() {
            let dir = test_dirs::temp("delete-success-clears").unwrap();
            let state_path = dir.file("delete_queue.json");
            let tmp = temp_file(b"aaaa").await;
            let clock = Clock::new(1000);
            let mut deleter = DeleterImpl::new(DeleterArgs {
                clock: clock.clock(),
                snapshot_file: Some(snapshot_file(&state_path).await),
                ..DeleterArgs::default()
            });
            deleter
                .enqueue(make_job(tmp.file(), 1000, 0).await)
                .await
                .unwrap();

            deleter.sweep().await.unwrap();

            assert!(deleter.queue.is_empty());
            assert!(!tmp.file().exists());
            drop(deleter);

            let restored = DeleterImpl::new(DeleterArgs {
                snapshot_file: Some(snapshot_file(&state_path).await),
                ..DeleterArgs::default()
            });
            assert!(restored.queue.is_empty());
        }
    }

    mod backoff {
        use super::*;

        /// The backoff every test in this module runs under: 10s, doubling,
        /// capped at 40s — small enough to step a test clock over.
        const BACKOFF: cooldown::Backoff = cooldown::Backoff {
            base_secs: 10,
            growth_factor: 2,
            max_secs: 40,
        };

        /// A deleter on `clock` that backs off under [`BACKOFF`], persisting to
        /// `state_path` when one is given.
        async fn backoff_deleter(clock: &Clock, state_path: Option<&File>) -> DeleterImpl {
            let snapshot_file = match state_path {
                Some(path) => Some(snapshot_file(path).await),
                None => None,
            };
            DeleterImpl::new(DeleterArgs {
                clock: clock.clock(),
                backoff: BACKOFF,
                snapshot_file,
                ..DeleterArgs::default()
            })
        }

        #[tokio::test]
        async fn failure_defers_the_next_attempt() {
            let dir = test_dirs::temp("delete-backoff-defer").unwrap();
            let clock = Clock::new(1000);
            let mut deleter = backoff_deleter(&clock, None).await;
            deleter
                .enqueue(undeletable_dir_job(&dir, at(1000)).await)
                .await
                .unwrap();

            deleter.sweep().await.unwrap();
            let entry = deleter.queue.queue_entries()[0].clone();
            assert_eq!(entry.attempts, 1);
            assert_eq!(
                entry.next_attempt_at,
                Some(DateTime::from_timestamp(1010, 0).unwrap())
            );

            // still deferred: the next sweep must not touch it.
            clock.advance(9);
            deleter.sweep().await.unwrap();
            assert_eq!(deleter.queue.queue_entries()[0].attempts, 1);

            // due exactly at next_attempt_at.
            clock.advance(1);
            deleter.sweep().await.unwrap();
            assert_eq!(deleter.queue.queue_entries()[0].attempts, 2);
        }

        #[tokio::test]
        async fn the_delay_grows_and_caps() {
            let dir = test_dirs::temp("delete-backoff-growth").unwrap();
            let clock = Clock::new(1000);
            let mut deleter = backoff_deleter(&clock, None).await;
            deleter
                .enqueue(undeletable_dir_job(&dir, at(1000)).await)
                .await
                .unwrap();

            // 10, 20, 40, then pinned at the 40s cap.
            let mut at = 1000;
            for wait in [10, 20, 40, 40, 40] {
                deleter.sweep().await.unwrap();
                at += wait;
                assert_eq!(
                    deleter.queue.queue_entries()[0].next_attempt_at,
                    Some(DateTime::from_timestamp(at, 0).unwrap()),
                    "after a {wait}s wait"
                );
                clock.advance(wait as u64);
            }
        }

        #[tokio::test]
        async fn next_attempt_at_survives_a_reload() {
            let dir = test_dirs::temp("delete-backoff-reload").unwrap();
            let state_path = dir.file("delete_queue.json");
            let clock = Clock::new(1000);
            let mut deleter = backoff_deleter(&clock, Some(&state_path)).await;
            deleter
                .enqueue(undeletable_dir_job(&dir, at(1000)).await)
                .await
                .unwrap();
            deleter.sweep().await.unwrap();
            drop(deleter);

            let restored = backoff_deleter(&clock, Some(&state_path)).await;

            assert_eq!(
                restored.queue.queue_entries()[0].next_attempt_at,
                Some(DateTime::from_timestamp(1010, 0).unwrap())
            );
        }

        /// The sweep's loop budget comes from `count_ready` and its pops from
        /// `next_ready`; a deferred entry must be invisible to both or the two
        /// desynchronize.
        #[tokio::test]
        async fn count_ready_and_next_ready_agree_about_a_deferred_entry() {
            let dir = test_dirs::temp("delete-backoff-agree").unwrap();
            let clock = Clock::new(1000);
            let mut deleter = backoff_deleter(&clock, None).await;
            deleter
                .enqueue(undeletable_dir_job(&dir, at(1000)).await)
                .await
                .unwrap();
            deleter.sweep().await.unwrap();

            let deferred = DateTime::from_timestamp(1005, 0).unwrap();
            assert_eq!(deleter.queue.count_ready(deferred), 0);
            assert!(deleter.queue.next_ready(deferred).is_none());

            let due = DateTime::from_timestamp(1010, 0).unwrap();
            assert_eq!(deleter.queue.count_ready(due), 1);
            assert!(deleter.queue.next_ready(due).is_some());
        }
    }

    /// The TTL is counted in server time, or on the monotonic clock until
    /// server time is known, and never on the robot's system clock.
    mod expiry {
        use super::*;
        use crate::data_uploads::retention::queue::QueueEntry;
        use std::time::Duration;
        use uuid::Uuid;

        /// 2026-10-20T00:00:00Z, a plausible server time.
        const SERVER: i64 = 1_792_454_400;

        /// A deleter on `clock` persisting to `state_path`.
        async fn persisted(clock: &Clock, state_path: &File) -> DeleterImpl {
            DeleterImpl::new(DeleterArgs {
                clock: clock.clock(),
                backoff: NO_BACKOFF,
                snapshot_file: Some(snapshot_file(state_path).await),
                ..DeleterArgs::default()
            })
        }

        fn only_expiry(deleter: &DeleterImpl) -> Expiry {
            deleter.queue.entries()[0].expiry
        }

        #[tokio::test]
        async fn enqueue_stamps_a_server_deadline_when_server_time_is_known() {
            let tmp = temp_file(b"aaaa").await;
            let clock = Clock::new(1000);
            clock.observe_server(SERVER);
            let mut deleter = deleter(&clock);

            deleter
                .enqueue(make_job(tmp.file(), 1000, 500).await)
                .await
                .unwrap();

            assert_eq!(
                only_expiry(&deleter),
                Expiry {
                    server_expires_at: Some(at(SERVER + 500)),
                    remaining_ttl_secs: Some(500),
                }
            );
        }

        // The producer's expiry is never trusted: the TTL starts at enqueue.
        #[tokio::test]
        async fn enqueue_replaces_a_producer_expiry() {
            let tmp = temp_file(b"aaaa").await;
            let clock = Clock::new(1000);
            let mut deleter = deleter(&clock);
            let job = Job {
                expiry: Expiry {
                    server_expires_at: Some(at(0)),
                    remaining_ttl_secs: Some(0),
                },
                ..make_job(tmp.file(), 1000, 500).await
            };

            deleter.enqueue(job).await.unwrap();
            deleter.sweep().await.unwrap();

            assert!(tmp.file().exists());
            assert_eq!(only_expiry(&deleter), Expiry::start(500, None));
        }

        // The pre-NTP case: a file observed while the system clock read 1970
        // is not deleted when NTP moves the clock to the real date.
        #[tokio::test]
        async fn a_system_clock_jump_never_deletes_early() {
            let tmp = temp_file(b"aaaa").await;
            let clock = Clock::new(1000);
            let mut deleter = deleter(&clock);
            deleter
                .enqueue(make_job(tmp.file(), 1000, 500).await)
                .await
                .unwrap();

            clock.set_system(SERVER);
            deleter.sweep().await.unwrap();
            assert!(tmp.file().exists());

            clock.advance(499);
            deleter.sweep().await.unwrap();
            assert!(tmp.file().exists());

            clock.advance(1);
            deleter.sweep().await.unwrap();
            assert!(!tmp.file().exists());
        }

        // A system clock that is ahead of the server's time does not shorten a
        // TTL that has a server deadline.
        #[tokio::test]
        async fn a_server_deadline_ignores_the_system_clock() {
            let tmp = temp_file(b"aaaa").await;
            let clock = Clock::new(SERVER);
            clock.observe_server(SERVER);
            let mut deleter = deleter(&clock);
            deleter
                .enqueue(make_job(tmp.file(), SERVER, 500).await)
                .await
                .unwrap();

            clock.set_system(SERVER + 10_000);
            deleter.sweep().await.unwrap();
            assert!(tmp.file().exists());

            clock.advance(500);
            deleter.sweep().await.unwrap();
            assert!(!tmp.file().exists());
        }

        #[tokio::test]
        async fn a_job_gets_a_server_deadline_once_server_time_is_known() {
            let tmp = temp_file(b"aaaa").await;
            let clock = Clock::new(1000);
            let mut deleter = deleter(&clock);
            deleter
                .enqueue(make_job(tmp.file(), 1000, 500).await)
                .await
                .unwrap();
            clock.advance(100);
            deleter.sweep().await.unwrap();
            assert_eq!(only_expiry(&deleter).server_expires_at, None);

            clock.observe_server(SERVER);
            deleter.sweep().await.unwrap();

            // 100s of the TTL were already counted on the monotonic clock.
            assert_eq!(
                only_expiry(&deleter).server_expires_at,
                Some(at(SERVER + 400))
            );
        }

        // Restarted before hearing from the server: the countdown resumes from
        // the TTL saved at the latest checkpoint. Downtime does not count.
        #[tokio::test]
        async fn a_restart_without_server_time_resumes_from_the_checkpoint() {
            let dir = test_dirs::temp("delete-expiry-restart-offline").unwrap();
            let state_path = dir.file("delete_queue.json");
            let tmp = temp_file(b"aaaa").await;
            let clock = Clock::new(1000);
            let mut deleter = persisted(&clock, &state_path).await;
            deleter
                .enqueue(make_job(tmp.file(), 1000, 3600).await)
                .await
                .unwrap();
            clock.advance(1200);
            deleter.sweep().await.unwrap();
            drop(deleter);

            let clock = clock.restart();
            clock.set_system(1);
            let mut restored = persisted(&clock, &state_path).await;
            assert_eq!(only_expiry(&restored).remaining_ttl_secs, Some(2400));

            clock.advance(2399);
            restored.sweep().await.unwrap();
            assert!(tmp.file().exists());

            clock.advance(1);
            restored.sweep().await.unwrap();
            assert!(!tmp.file().exists());
        }

        // Restarted after the deadline passed in server time: the TTL keeps its
        // calendar meaning, so time the robot was off counts.
        #[tokio::test]
        async fn a_restart_with_server_time_keeps_calendar_days() {
            let dir = test_dirs::temp("delete-expiry-restart-online").unwrap();
            let state_path = dir.file("delete_queue.json");
            let tmp = temp_file(b"aaaa").await;
            let clock = Clock::new(SERVER);
            clock.observe_server(SERVER);
            let mut deleter = persisted(&clock, &state_path).await;
            deleter
                .enqueue(make_job(tmp.file(), SERVER, 3600).await)
                .await
                .unwrap();
            drop(deleter);

            // the robot was off for an hour and booted at 1970.
            let clock = clock.restart();
            clock.set_system(1);
            let mut restored = persisted(&clock, &state_path).await;
            restored.sweep().await.unwrap();
            assert!(
                tmp.file().exists(),
                "deleted before hearing from the server"
            );

            clock.observe_server(SERVER + 3600);
            restored.sweep().await.unwrap();
            assert!(!tmp.file().exists());
        }

        // A job saved before expiries existed carries only a system-clock
        // stamp, which cannot be trusted, so its TTL starts over.
        #[tokio::test]
        async fn a_job_saved_without_an_expiry_starts_its_ttl_over() {
            let dir = test_dirs::temp("delete-expiry-legacy").unwrap();
            let state_path = dir.file("delete_queue.json");
            let tmp = temp_file(b"aaaa").await;
            let mut snapshot = snapshot_file(&state_path).await;
            snapshot
                .patch(DeleteQueueSnapshot {
                    entries: vec![QueueEntry {
                        id: Uuid::new_v4(),
                        job: make_job(tmp.file(), 1000, 500).await,
                        attempts: 0,
                        next_attempt_at: None,
                    }],
                })
                .await
                .unwrap();
            let clock = Clock::new(SERVER);
            let mut deleter = persisted(&clock, &state_path).await;

            deleter.sweep().await.unwrap();
            assert!(tmp.file().exists());
            assert_eq!(only_expiry(&deleter), Expiry::start(500, None));

            clock.advance(500);
            deleter.sweep().await.unwrap();
            assert!(!tmp.file().exists());
        }

        #[tokio::test]
        async fn the_remaining_ttl_is_saved_at_each_checkpoint() {
            let dir = test_dirs::temp("delete-expiry-checkpoint").unwrap();
            let state_path = dir.file("delete_queue.json");
            let tmp = temp_file(b"aaaa").await;
            let clock = Clock::new(1000);
            let mut deleter = DeleterImpl::new(DeleterArgs {
                clock: clock.clock(),
                snapshot_file: Some(snapshot_file(&state_path).await),
                checkpoint_interval: Duration::from_secs(600),
                ..DeleterArgs::default()
            });
            deleter
                .enqueue(make_job(tmp.file(), 1000, 3600).await)
                .await
                .unwrap();
            let saved = |path: File| async move {
                snapshot_file(&path).await.read().entries[0]
                    .job
                    .expiry
                    .remaining_ttl_secs
            };

            // the first sweep checkpoints
            clock.advance(100);
            deleter.sweep().await.unwrap();
            assert_eq!(saved(state_path.clone()).await, Some(3500));

            // between checkpoints nothing is written
            clock.advance(300);
            deleter.sweep().await.unwrap();
            assert_eq!(saved(state_path.clone()).await, Some(3500));

            clock.advance(300);
            deleter.sweep().await.unwrap();
            assert_eq!(saved(state_path.clone()).await, Some(2900));
        }

        // A partly elapsed second is saved as a whole one, so a restart never
        // shortens the TTL.
        #[test]
        fn a_checkpoint_rounds_the_remaining_ttl_up() {
            let mut expiry = Expiry::default();
            let mut refresh = super::super::Refresh::default();

            let changed = DeleterImpl::update_expiry(
                &mut expiry,
                Duration::from_millis(1500),
                None,
                true,
                &mut refresh,
            );

            assert!(changed);
            assert_eq!(expiry.remaining_ttl_secs, Some(2));
        }
    }

    mod persistence {
        use super::*;

        #[tokio::test]
        async fn attempts_survive_a_restart() {
            let dir = test_dirs::temp("delete-attempts-restart").unwrap();
            let state_path = dir.file("delete_queue.json");
            let clock = Clock::new(1000);
            let mut deleter = DeleterImpl::new(DeleterArgs {
                clock: clock.clock(),
                backoff: NO_BACKOFF,
                snapshot_file: Some(snapshot_file(&state_path).await),
                ..DeleterArgs::default()
            });
            deleter
                .enqueue(undeletable_dir_job(&dir, at(1000)).await)
                .await
                .unwrap();
            deleter.sweep().await.unwrap();
            deleter.sweep().await.unwrap();
            drop(deleter);

            // no injected clock: the wedged job's TTL is zero, so it is due
            // at once. The cap is tightened on the rebuild so
            // the restored entry's third attempt is its last.
            let mut restored = DeleterImpl::new(DeleterArgs {
                attempts: 3,
                snapshot_file: Some(snapshot_file(&state_path).await),
                ..DeleterArgs::default()
            });
            assert_eq!(restored.queue.queue_entries()[0].attempts, 2);

            restored.sweep().await.unwrap();
            assert!(restored.queue.is_empty());
        }

        #[tokio::test]
        async fn dropped_entry_is_absent_from_the_persisted_snapshot() {
            let dir = test_dirs::temp("delete-attempts-drop-persist").unwrap();
            let state_path = dir.file("delete_queue.json");
            let clock = Clock::new(1000);
            let mut deleter = DeleterImpl::new(DeleterArgs {
                clock: clock.clock(),
                attempts: 1,
                snapshot_file: Some(snapshot_file(&state_path).await),
                ..DeleterArgs::default()
            });
            deleter
                .enqueue(undeletable_dir_job(&dir, at(1000)).await)
                .await
                .unwrap();

            deleter.sweep().await.unwrap();
            drop(deleter);

            let restored = DeleterImpl::new(DeleterArgs {
                snapshot_file: Some(snapshot_file(&state_path).await),
                ..DeleterArgs::default()
            });
            assert!(restored.queue.is_empty());
        }
    }
}
