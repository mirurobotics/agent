// standard crates
use std::fs::Metadata;
use std::sync::Arc;
use std::time::SystemTime;

// internal crates
use crate::data_uploads::retention::{
    errors::*,
    job::Job,
    queue::{DeleteQueueSnapshotFile, Queue, QueueEntry},
};
use crate::filesys::{errors::FileSysErr, files};
use crate::trace;

// external crates
use chrono::{DateTime, Utc};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

const DEFAULT_QUEUE_CAPACITY: usize = 4096;
const DEFAULT_ATTEMPTS: u32 = 10;

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
    /// Total sweep attempts per job before it is dropped.
    pub attempts: u32,
}

impl Default for DeleterArgs {
    fn default() -> Self {
        Self {
            now_fn: Arc::new(Utc::now),
            queue_capacity: DEFAULT_QUEUE_CAPACITY,
            snapshot_file: None,
            attempts: DEFAULT_ATTEMPTS,
        }
    }
}

pub struct SingleThreadDeleter {
    queue: Queue,
    now_fn: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
    attempts: u32,
}

/// Outcome of considering one queued job during a sweep.
enum SweepOutcome {
    /// A failure whose class might resolve on its own; consumes one attempt.
    CountedRetry,
    /// A failure whose class will never resolve for this recorded path;
    /// drop immediately rather than burning the whole attempt budget.
    TerminalFailure,
    /// File was deleted; drop the job.
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
            attempts: args.attempts,
        }
    }

    fn len(&self) -> usize {
        self.queue.len()
    }

    async fn enqueue(&mut self, job: Job) -> Result<(), DeleteErr> {
        self.queue.enqueue(job).await
    }

    async fn sweep(&mut self) -> Result<(), DeleteErr> {
        let now = (self.now_fn)();
        // Budget: one visit per entry that is due at `now`. A retried entry is
        // requeued at the tail, behind every entry not yet visited, so this
        // budget is exactly enough to visit each due entry once and never
        // twice.
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
        Ok(())
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
                warn!(
                    "delete: failed to stat {}: {err:?}; classifying failure",
                    entry.file
                );
                Err(Self::classify(&err))
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
                warn!(
                    "delete: failed to hash {}: {err:?}; classifying failure",
                    entry.file
                );
                Some(Self::classify(&err))
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
                    "delete: failed to delete {}: {err:?}; classifying failure",
                    entry.file
                );
                Self::classify(&err)
            }
        }
    }

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
            Some(
                PermissionDenied | ReadOnlyFilesystem | IsADirectory | NotADirectory
                | InvalidFilename,
            ) => SweepOutcome::TerminalFailure,
            _ => SweepOutcome::CountedRetry,
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

    /// Logs the ordinal of the failing sweep (`attempts + 1`); the terminal
    /// path consumes no attempt, so the field itself is one short.
    fn log_terminal_drop(entry: &QueueEntry) {
        error!(
            "delete: giving up on {} on attempt {} after a permanent filesystem \
             failure (rule {}, deployment {}, digest {}); the file is left on \
             disk and the agent will not retry it",
            entry.job.file,
            entry.attempts + 1,
            entry.job.file_rule_id,
            entry.job.deployment_id,
            entry.job.digest
        );
    }
}

// =================================== TRAIT ======================================= //
// `-> impl Future + Send` (not `async fn`) so callers awaiting a generic
// `D: DeleterExt` inside their own `Send` futures can prove those futures
// `Send`.
//
// an async, actor-round-tripping is_empty would be dead weight next to len()
#[allow(clippy::len_without_is_empty)]
pub trait DeleterExt: Send + Sync {
    fn enqueue(&self, job: Job) -> impl std::future::Future<Output = Result<(), DeleteErr>> + Send;
    /// Run one deletion pass over the queued jobs.
    fn sweep(&self) -> impl std::future::Future<Output = Result<(), DeleteErr>> + Send;
    /// The number of queued jobs, including one currently being swept: a
    /// selected job stays queued until its sweep resolves it.
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
/// by an external driver, not by this type. A sweep may safely be
/// interleaved with commands: a selected job never leaves the queue until
/// it resolves, so a persist triggered by another command cannot write the
/// in-flight job out of the snapshot.
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
    use super::{DeleterArgs, SingleThreadDeleter, SweepOutcome};
    use crate::data_uploads::retention::job::Job;
    use crate::data_uploads::retention::queue::{DeleteQueueSnapshot, DeleteQueueSnapshotFile};
    use crate::filesys::errors::{
        DeleteFileErr, FileMetadataErr, FileSysErr, OpenFileErr, PathExistsErr, ReadFileErr,
    };
    use crate::filesys::{dirs, files, Dir, File, PathExt, WriteOptions};
    use crate::trace;

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

    /// A comparable name for a sweep outcome. `SweepOutcome` deliberately
    /// derives neither `Debug` nor `PartialEq`, so tests compare this label —
    /// which also makes a failure report the outcome it actually got.
    fn outcome_label(outcome: &SweepOutcome) -> &'static str {
        match outcome {
            SweepOutcome::CountedRetry => "counted-retry",
            SweepOutcome::TerminalFailure => "terminal-failure",
            SweepOutcome::Deleted => "deleted",
            SweepOutcome::AlreadyGone => "already-gone",
            SweepOutcome::Changed => "changed",
        }
    }

    /// Two symlinks pointing at each other. `stat` and `open` on either fail
    /// with ELOOP, which has no `ErrorKind` variant nameable on stable 1.97.0
    /// and therefore classifies as a counted retry, not a terminal failure.
    /// Built with `std::os::unix::fs::symlink` rather than
    /// `files::create_symlink` because the latter asserts the target exists.
    fn symlink_loop(dir: &Dir) -> File {
        let a = dir.file("loop-a");
        let b = dir.file("loop-b");
        std::os::unix::fs::symlink(b.path(), a.path()).unwrap();
        std::os::unix::fs::symlink(a.path(), b.path()).unwrap();
        a
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
        }
    }

    mod enqueue {
        use super::*;
        use crate::data_uploads::retention::errors::DeleteErr;

        #[tokio::test]
        async fn appends_the_job() {
            let tmp = temp_file(b"aaaa").await;
            let clock = Clock::new(1000);
            let mut deleter = deleter(&clock);
            let job = make_job(tmp.file(), 1000, 0).await;

            deleter.enqueue(job.clone()).await.unwrap();

            assert_eq!(deleter.queue.entries(), [job]);
        }

        #[tokio::test]
        async fn persists_to_disk() {
            let dir = dirs::temp("delete-enqueue-persist").unwrap();
            let state_path = dir.file("delete_queue.json");
            let tmp = temp_file(b"aaaa").await;
            let clock = Clock::new(1000);
            let mut deleter = SingleThreadDeleter::new(DeleterArgs {
                now_fn: Arc::new(clock.now_fn()),
                snapshot_file: Some(snapshot_file(&state_path).await),
                ..DeleterArgs::default()
            });
            let job = make_job(tmp.file(), 1000, 500).await;
            deleter.enqueue(job.clone()).await.unwrap();
            drop(deleter);

            let restored = SingleThreadDeleter::new(DeleterArgs {
                snapshot_file: Some(snapshot_file(&state_path).await),
                ..DeleterArgs::default()
            });
            assert_eq!(restored.queue.entries(), [job]);
        }

        #[tokio::test]
        async fn full_queue_returns_queue_full_err() {
            let tmp_a = temp_file(b"aaaa").await;
            let tmp_b = temp_file(b"bbbb").await;
            let clock = Clock::new(1000);
            let mut deleter = SingleThreadDeleter::new(DeleterArgs {
                now_fn: Arc::new(clock.now_fn()),
                queue_capacity: 1,
                ..DeleterArgs::default()
            });
            let first = make_job(tmp_a.file(), 1000, 0).await;
            deleter.enqueue(first.clone()).await.unwrap();

            let err = deleter
                .enqueue(make_job(tmp_b.file(), 1000, 0).await)
                .await
                .unwrap_err();
            let DeleteErr::QueueFullErr(full) = err else {
                panic!("expected QueueFullErr, got: {err:?}");
            };
            assert_eq!(full.capacity, 1);
            assert_eq!(full.file, tmp_b.file().to_string());
        }

        #[tokio::test]
        async fn persist_failure_is_swallowed() {
            let dir = dirs::temp("delete-enqueue-persist-fail").unwrap();
            let state_path = dir.file("delete_queue.json");
            let tmp = temp_file(b"aaaa").await;
            let clock = Clock::new(1000);
            let mut deleter = SingleThreadDeleter::new(DeleterArgs {
                now_fn: Arc::new(clock.now_fn()),
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
            assert_eq!(deleter.queue.entries(), [job]);
        }
    }

    mod classify {
        use super::*;

        fn io(kind: std::io::ErrorKind) -> Box<std::io::Error> {
            Box::new(std::io::Error::from(kind))
        }

        #[test]
        fn permission_denied_is_terminal() {
            let err = FileSysErr::DeleteFileErr(DeleteFileErr {
                source: io(std::io::ErrorKind::PermissionDenied),
                file: File::new("/data/a.log"),
                trace: trace!(),
            });
            assert_eq!(
                outcome_label(&SingleThreadDeleter::classify(&err)),
                "terminal-failure"
            );
        }

        #[test]
        fn read_only_filesystem_is_terminal() {
            let err = FileSysErr::FileMetadataErr(FileMetadataErr {
                file: File::new("/data/a.log"),
                source: io(std::io::ErrorKind::ReadOnlyFilesystem),
                trace: trace!(),
            });
            assert_eq!(
                outcome_label(&SingleThreadDeleter::classify(&err)),
                "terminal-failure"
            );
        }

        #[test]
        fn is_a_directory_is_terminal() {
            let err = FileSysErr::OpenFileErr(OpenFileErr {
                source: io(std::io::ErrorKind::IsADirectory),
                file: File::new("/data/a.log"),
                trace: trace!(),
            });
            assert_eq!(
                outcome_label(&SingleThreadDeleter::classify(&err)),
                "terminal-failure"
            );
        }

        #[test]
        fn not_a_directory_is_terminal() {
            let err = FileSysErr::ReadFileErr(ReadFileErr {
                source: io(std::io::ErrorKind::NotADirectory),
                file: File::new("/data/a.log"),
                trace: trace!(),
            });
            assert_eq!(
                outcome_label(&SingleThreadDeleter::classify(&err)),
                "terminal-failure"
            );
        }

        #[test]
        fn invalid_filename_is_terminal() {
            let err = FileSysErr::DeleteFileErr(DeleteFileErr {
                source: io(std::io::ErrorKind::InvalidFilename),
                file: File::new("/data/a.log"),
                trace: trace!(),
            });
            assert_eq!(
                outcome_label(&SingleThreadDeleter::classify(&err)),
                "terminal-failure"
            );
        }

        #[test]
        fn unclassified_io_error_is_counted() {
            let err = FileSysErr::ReadFileErr(ReadFileErr {
                source: io(std::io::ErrorKind::TimedOut),
                file: File::new("/data/a.log"),
                trace: trace!(),
            });
            assert_eq!(
                outcome_label(&SingleThreadDeleter::classify(&err)),
                "counted-retry"
            );
        }

        /// Pins `io_kind`'s catch-all arm: a filesystem error carrying no
        /// `io::Error` is unclassifiable and therefore counted.
        #[test]
        fn error_without_an_io_source_is_counted() {
            let err = FileSysErr::PathExistsErr(PathExistsErr {
                path: std::path::PathBuf::from("/data/a.log"),
                trace: trace!(),
            });
            assert_eq!(
                outcome_label(&SingleThreadDeleter::classify(&err)),
                "counted-retry"
            );
        }

        /// ELOOP is the real-filesystem failure the counted-retry tests below
        /// are built on. Assert on the classification, never on the
        /// `ErrorKind` variant name — `FilesystemLoop` is behind the unstable
        /// `io_error_more` feature on 1.97.0 and naming it is E0658. If a
        /// platform ever maps ELOOP to a terminal kind, this is the single
        /// place to re-point.
        #[tokio::test]
        async fn symlink_loop_is_counted() {
            let dir = dirs::temp("delete-classify-eloop").unwrap();
            let head = symlink_loop(&dir);

            let err = files::metadata(&head).await.unwrap_err();

            assert_eq!(
                outcome_label(&SingleThreadDeleter::classify(&err)),
                "counted-retry"
            );
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
            assert_eq!(deleter.queue.entries(), [waiting_job]);
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

        // ENOTDIR: the recorded path's parent is a file, so the stat can never
        // succeed for this path.
        #[tokio::test]
        async fn stat_permanent_failure_drops_entry() {
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
            deleter.enqueue(record).await.unwrap();

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

        // EISDIR: the recorded path is a directory, so the hash read fails
        // permanently; the pass must drop the entry, not panic.
        #[tokio::test]
        async fn hash_permanent_failure_drops_entry() {
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
            deleter.enqueue(record).await.unwrap();

            deleter.sweep().await.unwrap();

            assert!(deleter.queue.is_empty());
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
        // directories, so files::delete fails permanently. The deleter keeps
        // the default budget of 10 attempts.
        #[tokio::test]
        async fn terminal_failure_drops_job_without_burning_attempts() {
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
            deleter.enqueue(record).await.unwrap();

            deleter.sweep().await.unwrap();

            assert!(deleter.queue.is_empty());
            assert!(target.exists());
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

            // the due job was removed-and-persisted (Queue::remove) before the
            // waiting job was considered, so a rebuild sees only the waiting
            // job.
            let restored = SingleThreadDeleter::new(DeleterArgs {
                snapshot_file: Some(snapshot_file(&state_path).await),
                ..DeleterArgs::default()
            });
            assert!(!due.file().exists());
            assert!(waiting.file().exists());
            assert_eq!(restored.queue.entries(), [waiting_job]);
        }

        // A not-due entry is never selected, so a sweep leaves it exactly
        // where it is — same position, same id — even while a due entry
        // behind it resolves and rewrites the snapshot.
        #[tokio::test]
        async fn not_due_entries_are_left_untouched() {
            let dir = dirs::temp("delete-sweep-not-due").unwrap();
            let state_path = dir.file("delete_queue.json");
            let waiting_a = temp_file(b"aaaa").await;
            let waiting_b = temp_file(b"bbbb").await;
            let due = temp_file(b"cccc").await;
            let clock = Clock::new(1000);
            let mut deleter = SingleThreadDeleter::new(DeleterArgs {
                now_fn: Arc::new(clock.now_fn()),
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

        /// The hash step's already-gone arm. There is no deterministic
        /// real-filesystem route to it through `sweep`: reaching the hash
        /// requires a successful stat, and `files::metadata` follows symlinks,
        /// so any path that ENOENTs on open also ENOENTs on stat and is caught
        /// by `stat_file` first. It is a genuine TOCTOU window — the file
        /// vanishes between the stat and the re-hash — so the helper is
        /// exercised directly.
        #[tokio::test]
        async fn vanished_file_at_the_hash_step_is_already_gone() {
            let job = wedged_job(File::new("/nonexistent/miru-delete-test/a.log"));

            let outcome = SingleThreadDeleter::check_digest_mismatch(&job).await;

            assert_eq!(outcome.as_ref().map(outcome_label), Some("already-gone"));
        }

        /// The sibling of the above: a hash failure that is *not* a vanished
        /// file still consumes an attempt rather than dropping the job.
        #[tokio::test]
        async fn hash_of_a_wedged_path_is_counted() {
            let dir = dirs::temp("delete-hash-eloop").unwrap();
            let job = wedged_job(symlink_loop(&dir));

            let outcome = SingleThreadDeleter::check_digest_mismatch(&job).await;

            assert_eq!(outcome.as_ref().map(outcome_label), Some("counted-retry"));
        }

        #[tokio::test]
        async fn counted_failure_increments_attempts() {
            let dir = dirs::temp("delete-attempts-counted").unwrap();
            let clock = Clock::new(1000);
            let mut deleter = deleter(&clock);
            deleter
                .enqueue(wedged_job(symlink_loop(&dir)))
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
            let dir = dirs::temp("delete-attempts-cap").unwrap();
            let clock = Clock::new(1000);
            let mut deleter = SingleThreadDeleter::new(DeleterArgs {
                now_fn: Arc::new(clock.now_fn()),
                attempts: 3,
                ..DeleterArgs::default()
            });
            deleter
                .enqueue(wedged_job(symlink_loop(&dir)))
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

            let dir = dirs::temp("delete-attempts-default").unwrap();
            let clock = Clock::new(1000);
            let mut deleter = deleter(&clock);
            deleter
                .enqueue(wedged_job(symlink_loop(&dir)))
                .await
                .unwrap();

            for i in 1..=10 {
                deleter.sweep().await.unwrap();
                assert_eq!(deleter.queue.is_empty(), i == 10, "after sweep {i}");
            }
        }

        #[tokio::test]
        async fn successful_delete_clears_the_entry() {
            let dir = dirs::temp("delete-success-clears").unwrap();
            let state_path = dir.file("delete_queue.json");
            let tmp = temp_file(b"aaaa").await;
            let clock = Clock::new(1000);
            let mut deleter = SingleThreadDeleter::new(DeleterArgs {
                now_fn: Arc::new(clock.now_fn()),
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

            let restored = SingleThreadDeleter::new(DeleterArgs {
                snapshot_file: Some(snapshot_file(&state_path).await),
                ..DeleterArgs::default()
            });
            assert!(restored.queue.is_empty());
        }
    }

    mod persistence {
        use super::*;

        /// A restart is not a fresh budget: the counter is persisted with the
        /// entry, so a job that has already burned attempts resumes where it
        /// left off. `dir` must outlive the whole test — dropping it deletes
        /// the symlinks and turns the failure into an already-gone drop.
        #[tokio::test]
        async fn attempts_survive_a_restart() {
            let dir = dirs::temp("delete-attempts-restart").unwrap();
            let state_path = dir.file("delete_queue.json");
            let clock = Clock::new(1000);
            let mut deleter = SingleThreadDeleter::new(DeleterArgs {
                now_fn: Arc::new(clock.now_fn()),
                snapshot_file: Some(snapshot_file(&state_path).await),
                ..DeleterArgs::default()
            });
            deleter
                .enqueue(wedged_job(symlink_loop(&dir)))
                .await
                .unwrap();
            deleter.sweep().await.unwrap();
            deleter.sweep().await.unwrap();
            drop(deleter);

            // no injected clock: the wedged job's due_at is in 1970, so the
            // wall clock finds it due. The cap is tightened on the rebuild so
            // the restored entry's third attempt is its last.
            let mut restored = SingleThreadDeleter::new(DeleterArgs {
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
            let dir = dirs::temp("delete-attempts-drop-persist").unwrap();
            let state_path = dir.file("delete_queue.json");
            let clock = Clock::new(1000);
            let mut deleter = SingleThreadDeleter::new(DeleterArgs {
                now_fn: Arc::new(clock.now_fn()),
                attempts: 1,
                snapshot_file: Some(snapshot_file(&state_path).await),
                ..DeleterArgs::default()
            });
            deleter
                .enqueue(wedged_job(symlink_loop(&dir)))
                .await
                .unwrap();

            deleter.sweep().await.unwrap();
            drop(deleter);

            let restored = SingleThreadDeleter::new(DeleterArgs {
                snapshot_file: Some(snapshot_file(&state_path).await),
                ..DeleterArgs::default()
            });
            assert!(restored.queue.is_empty());
        }
    }
}
