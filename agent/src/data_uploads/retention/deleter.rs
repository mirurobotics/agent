// standard crates
use std::fs::Metadata;
use std::sync::Arc;
use std::time::SystemTime;

// internal crates
use crate::data_uploads::retention::{
    errors::*,
    job::Job,
    queue::{DeleteQueueSnapshotFile, Queue},
};
use crate::filesys::{errors::FileSysErr, files};
use crate::trace;

// external crates
use chrono::{DateTime, Utc};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

const DEFAULT_QUEUE_CAPACITY: usize = 4096;

macro_rules! dispatch {
    ($op:expr, $respond_to:expr, $msg:expr) => {{
        let result = $op;
        if $respond_to.send(result).is_err() {
            error!($msg);
        }
    }};
}

// ======================== SINGLE-THREADED IMPLEMENTATION ========================= //
pub struct DeleterArgs {
    pub now_fn: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
    pub queue_capacity: usize,
    pub snapshot_file: Option<DeleteQueueSnapshotFile>,
}

impl Default for DeleterArgs {
    fn default() -> Self {
        Self {
            now_fn: Arc::new(Utc::now),
            queue_capacity: DEFAULT_QUEUE_CAPACITY,
            snapshot_file: None,
        }
    }
}

pub struct SingleThreadDeleter {
    queue: Queue,
    now_fn: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
}

/// Outcome of considering one queued job during a sweep.
enum SweepOutcome {
    /// Not yet due; requeue.
    NotDue,
    /// Transient stat/hash/delete failure; requeue and try next sweep.
    Retry,
    /// File was deleted.
    Deleted,
    /// File is already gone; drop the job.
    AlreadyGone,
    /// On-disk identity no longer matches the tagged file; drop without deleting.
    Changed,
}

impl SingleThreadDeleter {
    pub fn new(args: DeleterArgs) -> Self {
        let queue = match args.snapshot_file {
            Some(snapshot_file) => Queue::from_snapshot(args.queue_capacity, snapshot_file),
            None => Queue::new(args.queue_capacity),
        };
        Self {
            queue,
            now_fn: args.now_fn,
        }
    }

    fn len(&self) -> usize {
        self.queue.len()
    }

    async fn enqueue(&mut self, job: Job) -> Result<(), DeleteErr> {
        self.queue.enqueue(job)?;
        self.queue.persist().await;
        Ok(())
    }

    /// Walk the queue one job at a time. Each job is popped into ownership
    /// so a drop cannot hit a different entry. A dropped job is persisted
    /// before the next is considered; a kept job is requeued at the tail.
    /// Per-entry failures are logged and never propagated — `sweep` always
    /// returns `Ok(())`.
    async fn sweep(&mut self) -> Result<(), DeleteErr> {
        let now = (self.now_fn)();
        let n = self.queue.len();
        for _ in 0..n {
            let Some(entry) = self.queue.pop_front() else {
                break;
            };
            match Self::sweep_entry(&entry, now).await {
                SweepOutcome::NotDue | SweepOutcome::Retry => {
                    self.queue.requeue(entry);
                }
                SweepOutcome::Deleted | SweepOutcome::AlreadyGone | SweepOutcome::Changed => {
                    self.queue.persist().await;
                }
            }
        }
        Ok(())
    }

    /// Consider one job: delete only when it is due and the on-disk file still
    /// matches the tagged size/mtime/digest.
    async fn sweep_entry(entry: &Job, now: DateTime<Utc>) -> SweepOutcome {
        if now < entry.due_at() {
            return SweepOutcome::NotDue;
        }
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
                warn!(
                    "delete: failed to stat {}: {err:?}; retrying next sweep",
                    entry.file
                );
                Err(SweepOutcome::Retry)
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
            Err(err) => {
                warn!(
                    "delete: failed to hash {}: {err:?}; retrying next sweep",
                    entry.file
                );
                Some(SweepOutcome::Retry)
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
                warn!(
                    "delete: failed to delete {}: {err:?}; retrying next sweep",
                    entry.file
                );
                SweepOutcome::Retry
            }
        }
    }
}

// =================================== TRAIT ======================================= //
// `-> impl Future + Send` (not `async fn`) so callers awaiting a generic
// `D: DeleterExt` inside their own `Send` futures — the upload executor — can
// prove those futures `Send` (the `TokenManagerExt`/`ObjectTransfer` pattern).
//
// an async, actor-round-tripping is_empty would be dead weight next to len()
#[allow(clippy::len_without_is_empty)]
pub trait DeleterExt: Send + Sync {
    /// Enqueue a job.
    fn enqueue(&self, job: Job) -> impl std::future::Future<Output = Result<(), DeleteErr>> + Send;
    /// Walk the queue one job at a time, persisting after each drop.
    fn sweep(&self) -> impl std::future::Future<Output = Result<(), DeleteErr>> + Send;
    /// The number of jobs in the queue.
    fn len(&self) -> impl std::future::Future<Output = Result<usize, DeleteErr>> + Send;
    /// Stop the actor. Queued jobs stay in the persisted snapshot and are
    /// re-seeded on the next spawn.
    fn shutdown(&self) -> impl std::future::Future<Output = Result<(), DeleteErr>> + Send;
}

// ================================== WORKER ======================================= //
pub(crate) enum Command {
    Enqueue {
        job: Job,
        respond_to: oneshot::Sender<Result<(), DeleteErr>>,
    },
    Sweep {
        respond_to: oneshot::Sender<Result<(), DeleteErr>>,
    },
    Len {
        respond_to: oneshot::Sender<Result<usize, DeleteErr>>,
    },
    Shutdown {
        respond_to: oneshot::Sender<Result<(), DeleteErr>>,
    },
}

pub(crate) struct Worker {
    deleter: SingleThreadDeleter,
    receiver: mpsc::Receiver<Command>,
}

impl Worker {
    pub(crate) async fn run(mut self) {
        while let Some(cmd) = self.receiver.recv().await {
            match cmd {
                Command::Shutdown { respond_to } => {
                    if let Err(e) = respond_to.send(Ok(())) {
                        error!("Actor failed to send shutdown response: {:?}", e);
                    }
                    break;
                }
                Command::Enqueue { job, respond_to } => {
                    dispatch!(
                        self.deleter.enqueue(job).await,
                        respond_to,
                        "Actor failed to send enqueue response"
                    );
                }
                Command::Sweep { respond_to } => {
                    dispatch!(
                        self.deleter.sweep().await,
                        respond_to,
                        "Actor failed to send sweep response"
                    );
                }
                Command::Len { respond_to } => {
                    dispatch!(
                        Ok(self.deleter.len()),
                        respond_to,
                        "Actor failed to send len response"
                    );
                }
            }
        }
    }
}

// ================================== HANDLE ======================================= //
/// Command handle to the [`SingleThreadDeleter`] actor. Reactive, not
/// self-scheduling: each [`sweep`](DeleterExt::sweep) call walks the queue
/// one job at a time. The cadence that drives repeated sweeps is imposed
/// by an external driver, not by this type.
#[derive(Debug)]
pub struct Deleter {
    sender: mpsc::Sender<Command>,
}

impl Deleter {
    pub fn spawn(
        buffer_size: usize,
        args: DeleterArgs,
    ) -> Result<(Self, JoinHandle<()>), DeleteErr> {
        let (sender, receiver) = mpsc::channel(buffer_size);
        let worker = Worker {
            deleter: SingleThreadDeleter::new(args),
            receiver,
        };
        let worker_handle = tokio::spawn(worker.run());
        Ok((Self { sender }, worker_handle))
    }

    async fn send_command<R>(
        &self,
        cmd: impl FnOnce(oneshot::Sender<R>) -> Command,
    ) -> Result<R, DeleteErr> {
        let (send, recv) = oneshot::channel();
        self.sender.send(cmd(send)).await.map_err(|e| {
            DeleteErr::SendActorMessageErr(SendActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })?;
        recv.await.map_err(|e| {
            DeleteErr::ReceiveActorMessageErr(ReceiveActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })
    }
}

impl DeleterExt for Deleter {
    async fn enqueue(&self, job: Job) -> Result<(), DeleteErr> {
        self.send_command(|tx| Command::Enqueue {
            job,
            respond_to: tx,
        })
        .await?
    }

    async fn sweep(&self) -> Result<(), DeleteErr> {
        self.send_command(|tx| Command::Sweep { respond_to: tx })
            .await?
    }

    async fn len(&self) -> Result<usize, DeleteErr> {
        self.send_command(|tx| Command::Len { respond_to: tx })
            .await?
    }

    async fn shutdown(&self) -> Result<(), DeleteErr> {
        self.send_command(|tx| Command::Shutdown { respond_to: tx })
            .await??;
        info!("delete: deleter shutdown complete");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    // standard crates
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::sync::Arc;

    // internal crates
    use super::{DeleterArgs, SingleThreadDeleter};
    use crate::data_uploads::retention::job::Job;
    use crate::data_uploads::retention::queue::{DeleteQueueSnapshot, DeleteQueueSnapshotFile};
    use crate::filesys::{dirs, files, File, PathExt, WriteOptions};

    // external crates
    use chrono::{DateTime, Utc};

    /// A controllable monotonic-ish clock for deterministic sweep tests. Holds
    /// the current time as epoch seconds in a shared atomic so a test can step
    /// it forward independently of wall-clock time. `now_fn()` produces the
    /// `Fn() -> DateTime<Utc>` closure that the deleter's injected `now_fn`
    /// expects.
    #[derive(Clone)]
    pub struct Clock {
        secs: Arc<AtomicI64>,
    }

    impl Clock {
        pub fn new(start_secs: i64) -> Self {
            Self {
                secs: Arc::new(AtomicI64::new(start_secs)),
            }
        }

        pub fn now_fn(&self) -> impl Fn() -> DateTime<Utc> {
            let secs = self.secs.clone();
            move || DateTime::from_timestamp(secs.load(Ordering::SeqCst), 0).unwrap()
        }

        pub fn advance(&self, secs: i64) {
            self.secs.fetch_add(secs, Ordering::SeqCst);
        }
    }

    // =============================== TEST HELPERS ================================= //

    /// A real on-disk temp file holding `contents`; the returned guard deletes
    /// it on drop.
    async fn temp_file(contents: &[u8]) -> files::TempFile {
        let tmp = files::temp("delete-sweep-test").unwrap();
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
        }
    }

    /// A deleter with the injected clock, default capacity, and no persistence.
    fn deleter(clock: &Clock) -> SingleThreadDeleter {
        SingleThreadDeleter::new(DeleterArgs {
            now_fn: Arc::new(clock.now_fn()),
            ..DeleterArgs::default()
        })
    }

    /// A persistence handle for the snapshot at `file`.
    async fn snapshot_file(file: &File) -> DeleteQueueSnapshotFile {
        DeleteQueueSnapshotFile::new_with_default(file.clone(), DeleteQueueSnapshot::default())
            .await
            .unwrap()
    }

    mod sweep {
        use super::*;

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

        #[tokio::test]
        async fn each_drop_is_persisted_before_the_next_job() {
            let dir = dirs::temp("delete-sweep-persist").unwrap();
            let state_path = dir.file("delete_queue.json");
            let due = temp_file(b"aaaa").await;
            let waiting = temp_file(b"bbbb").await;
            let clock = Clock::new(1000);
            let mut deleter = SingleThreadDeleter::new(DeleterArgs {
                now_fn: Arc::new(clock.now_fn()),
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

            // the due job was persisted-out before the waiting job was
            // considered, so a rebuild sees only the waiting job.
            let restored = SingleThreadDeleter::new(DeleterArgs {
                snapshot_file: Some(snapshot_file(&state_path).await),
                ..DeleterArgs::default()
            });
            assert!(!due.file().exists());
            assert!(waiting.file().exists());
            assert_eq!(restored.queue.entries(), [waiting_job]);
        }

        #[tokio::test]
        async fn positive_delay_entry_waits_for_due_at() {
            let tmp = temp_file(b"aaaa").await;
            let clock = Clock::new(1000);
            let mut deleter = deleter(&clock);
            let record = make_job(tmp.file(), 1000, 500).await;
            deleter.enqueue(record.clone()).await.unwrap();

            // not yet due: both 1000 and 1499 are before due_at (1500).
            deleter.sweep().await.unwrap();
            clock.advance(499);
            deleter.sweep().await.unwrap();
            assert_eq!(deleter.queue.entries(), [record]);
            assert!(tmp.file().exists());

            // due exactly at due_at.
            clock.advance(1);
            deleter.sweep().await.unwrap();
            assert!(deleter.queue.is_empty());
            assert!(!tmp.file().exists());
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

        // a hash failure (EISDIR: the recorded path is a directory, whose read
        // fails) keeps the entry for the next sweep and never panics the pass.
        #[tokio::test]
        async fn hash_failure_retains_entry() {
            let dir = dirs::temp("delete-hash-eisdir").unwrap();
            let target = File::new(dir.path().clone());
            let metadata = files::metadata(&target).await.unwrap();
            let clock = Clock::new(1000);
            let mut deleter = deleter(&clock);
            let record = Job {
                file: target.clone(),
                size: metadata.len(),
                digest: "sha256:unused".to_string(),
                // sentinel mtime: the re-stat mismatches, forcing the re-hash.
                mtime: DateTime::from_timestamp(1, 0).unwrap(),
                first_observed_at: DateTime::from_timestamp(1000, 0).unwrap(),
                last_observed_at: DateTime::from_timestamp(1000, 0).unwrap(),
                ttl_secs: 0,
                file_rule_id: "rule_1".to_string(),
                deployment_id: "dpl_1".to_string(),
            };
            deleter.enqueue(record.clone()).await.unwrap();

            deleter.sweep().await.unwrap();

            assert_eq!(deleter.queue.entries(), [record]);
            assert!(target.exists());
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

        // files::delete failure (EISDIR: the recorded path is a directory, and
        // unlink refuses directories) keeps the entry for the next sweep and
        // never panics the pass.
        #[tokio::test]
        async fn delete_failure_retains_entry() {
            let dir = dirs::temp("delete-eisdir").unwrap();
            let target = File::new(dir.path().clone());
            let metadata = files::metadata(&target).await.unwrap();
            let clock = Clock::new(1000);
            let mut deleter = deleter(&clock);
            let record = Job {
                file: target.clone(),
                size: metadata.len(),
                digest: "sha256:unused".to_string(),
                mtime: DateTime::<Utc>::from(metadata.modified().unwrap()),
                first_observed_at: DateTime::from_timestamp(1000, 0).unwrap(),
                last_observed_at: DateTime::from_timestamp(1000, 0).unwrap(),
                ttl_secs: 0,
                file_rule_id: "rule_1".to_string(),
                deployment_id: "dpl_1".to_string(),
            };
            deleter.enqueue(record.clone()).await.unwrap();

            deleter.sweep().await.unwrap();

            assert_eq!(deleter.queue.entries(), [record]);
            assert!(target.exists());
        }

        // a stat failure that is not NotFound (here ENOTDIR: the recorded
        // path's parent is a file) keeps the entry for the next sweep.
        #[tokio::test]
        async fn stat_failure_retains_entry() {
            let parent = temp_file(b"not a dir").await;
            let child = File::new(parent.file().path().join("child"));
            let clock = Clock::new(1000);
            let mut deleter = deleter(&clock);
            let record = Job {
                file: child,
                size: 4,
                digest: "sha256:unused".to_string(),
                mtime: DateTime::from_timestamp(900, 0).unwrap(),
                first_observed_at: DateTime::from_timestamp(1000, 0).unwrap(),
                last_observed_at: DateTime::from_timestamp(1000, 0).unwrap(),
                ttl_secs: 0,
                file_rule_id: "rule_1".to_string(),
                deployment_id: "dpl_1".to_string(),
            };
            deleter.enqueue(record.clone()).await.unwrap();

            deleter.sweep().await.unwrap();

            assert_eq!(deleter.queue.entries(), [record]);
        }
    }
}
