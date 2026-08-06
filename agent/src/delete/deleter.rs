// standard crates
use std::sync::Arc;
use std::time::SystemTime;

// internal crates
use crate::delete::{
    errors::*,
    queue::{DeleteQueueSnapshot, DeleteQueueSnapshotFile, PendingDelete},
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
    entries: Vec<PendingDelete>,
    capacity: usize,
    now_fn: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
    snapshot_file: Option<DeleteQueueSnapshotFile>,
}

impl SingleThreadDeleter {
    /// Seeds the queue from `args.snapshot_file`'s persisted entries (loaded in
    /// full even if they exceed `queue_capacity`; capacity only gates new
    /// enqueues, so an over-capacity backlog simply drains before it accepts
    /// more).
    pub fn new(args: DeleterArgs) -> Self {
        let entries = match &args.snapshot_file {
            Some(snapshot_file) => snapshot_file.read().entries.clone(),
            None => Vec::new(),
        };
        Self {
            entries,
            capacity: args.queue_capacity,
            now_fn: args.now_fn,
            snapshot_file: args.snapshot_file,
        }
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    /// Record `pending` as the queue's newest knowledge of its path: any
    /// existing entry for the same file is replaced (newest record wins), so
    /// the queue holds at most one entry per path. At capacity the enqueue is
    /// rejected with [`DeleteErr::QueueFullErr`] — the file simply stays on
    /// disk, the safe direction.
    async fn enqueue(&mut self, pending: PendingDelete) -> Result<(), DeleteErr> {
        self.entries.retain(|entry| entry.file != pending.file);
        if self.entries.len() >= self.capacity {
            warn!(
                "delete: queue is full (capacity {}); rejecting pending deletion for file {}",
                self.capacity, pending.file
            );
            return Err(DeleteErr::QueueFullErr(QueueFullErr {
                capacity: self.capacity,
                file: pending.file.to_string(),
                trace: trace!(),
            }));
        }
        self.entries.push(pending);
        self.persist_snapshot().await;
        info!(
            "delete: pending deletion enqueued; queue length {}",
            self.entries.len()
        );
        Ok(())
    }

    /// One pass over the queue: delete every due entry whose file is provably
    /// unchanged since upload; keep the rest. Per-entry failures are logged
    /// and never propagated — `sweep` always returns `Ok(())`.
    async fn sweep(&mut self) -> Result<(), DeleteErr> {
        let now = (self.now_fn)();
        let before = self.entries.len();
        let entries = std::mem::take(&mut self.entries);
        for entry in entries {
            if Self::sweep_entry(&entry, now).await {
                self.entries.push(entry);
            }
        }
        if self.entries.len() != before {
            self.persist_snapshot().await;
        }
        Ok(())
    }

    /// Whether `entry` should stay in the queue after this pass. Deletes the
    /// file only when it is due and its current size and mtime still match the
    /// record; every other outcome either drops the entry WITHOUT deleting
    /// (already gone, changed since upload) or keeps it for the next sweep
    /// (not yet due, stat or delete failure).
    async fn sweep_entry(entry: &PendingDelete, now: DateTime<Utc>) -> bool {
        if now < entry.due_at() {
            return true;
        }

        let metadata = match files::metadata(&entry.file).await {
            Ok(metadata) => metadata,
            Err(FileSysErr::PathDoesNotExistErr(_)) => {
                info!("delete: {} already gone; dropping entry", entry.file);
                return false;
            }
            Err(err) => {
                warn!(
                    "delete: failed to stat {}: {err:?}; retrying next sweep",
                    entry.file
                );
                return true;
            }
        };

        // Convert the stat's SystemTime before comparing, exactly as the
        // scanner does when it records the mtime (never compare a SystemTime
        // to a DateTime<Utc> directly).
        let mtime = DateTime::<Utc>::from(metadata.modified().unwrap_or(SystemTime::now()));
        if metadata.len() != entry.size || mtime != entry.mtime {
            info!(
                "delete: {} changed since upload; dropping without deleting",
                entry.file
            );
            return false;
        }

        match files::delete(&entry.file).await {
            Ok(()) => {
                info!(
                    "delete: deleted {} (rule {}, deployment {})",
                    entry.file, entry.upload_rule_id, entry.deployment_id
                );
                false
            }
            Err(err) => {
                warn!(
                    "delete: failed to delete {}: {err:?}; retrying next sweep",
                    entry.file
                );
                true
            }
        }
    }

    async fn persist_snapshot(&mut self) {
        let Some(snapshot_file) = self.snapshot_file.as_mut() else {
            return;
        };
        let snapshot = DeleteQueueSnapshot {
            entries: self.entries.clone(),
        };
        if let Err(err) = snapshot_file.patch(snapshot).await {
            warn!("delete: failed to persist delete queue: {err}");
        }
    }
}

// =================================== TRAIT ======================================= //
#[allow(async_fn_in_trait)]
// an async, actor-round-tripping is_empty would be dead weight next to len()
#[allow(clippy::len_without_is_empty)]
pub trait DeleterExt: Send + Sync {
    /// Record a pending deletion, replacing any existing entry for the same
    /// file.
    async fn enqueue(&self, pending: PendingDelete) -> Result<(), DeleteErr>;
    /// Perform one sweep pass over the queue.
    async fn sweep(&self) -> Result<(), DeleteErr>;
    /// The number of pending deletions in the queue.
    async fn len(&self) -> Result<usize, DeleteErr>;
    /// Stop the actor. Pending entries stay in the persisted snapshot and are
    /// re-seeded on the next spawn.
    async fn shutdown(&self) -> Result<(), DeleteErr>;
}

// ================================== WORKER ======================================= //
pub(crate) enum Command {
    Enqueue {
        pending: PendingDelete,
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
                Command::Enqueue {
                    pending,
                    respond_to,
                } => {
                    dispatch!(
                        self.deleter.enqueue(pending).await,
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
/// self-scheduling: each [`sweep`](DeleterExt::sweep) call performs exactly one
/// pass over the pending-deletion queue. The cadence that drives repeated
/// passes is imposed by an external driver, not by this type.
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
    async fn enqueue(&self, pending: PendingDelete) -> Result<(), DeleteErr> {
        self.send_command(|tx| Command::Enqueue {
            pending,
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
    use crate::delete::errors::DeleteErr;
    use crate::delete::queue::{DeleteQueueSnapshot, DeleteQueueSnapshotFile, PendingDelete};
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

    /// A `PendingDelete` for `file` whose size/mtime/digest reflect the file's
    /// current on-disk state.
    async fn pending(file: &File, eligible_secs: i64, delete_delay_secs: i64) -> PendingDelete {
        PendingDelete {
            file: file.clone(),
            size: files::size(file).await.unwrap(),
            mtime: DateTime::<Utc>::from(files::last_modified(file).await.unwrap()),
            digest: files::hash(file).await.unwrap(),
            eligible_at: DateTime::from_timestamp(eligible_secs, 0).unwrap(),
            delete_delay_secs,
            upload_rule_id: "rule_1".to_string(),
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

    mod due_at {
        use super::*;

        #[tokio::test]
        async fn adds_delay_and_clamps_negative_delays_to_zero() {
            let tmp = temp_file(b"aaaa").await;
            let record = pending(tmp.file(), 1000, 300).await;
            assert_eq!(record.due_at(), DateTime::from_timestamp(1300, 0).unwrap());

            let negative = PendingDelete {
                delete_delay_secs: -5,
                ..record
            };
            assert_eq!(
                negative.due_at(),
                DateTime::from_timestamp(1000, 0).unwrap()
            );
        }
    }

    mod enqueue {
        use super::*;

        #[tokio::test]
        async fn same_path_enqueue_replaces_older_record() {
            let tmp = temp_file(b"aaaa").await;
            let clock = Clock::new(1000);
            let mut deleter = deleter(&clock);
            let first = pending(tmp.file(), 1000, 100).await;
            let second = pending(tmp.file(), 1200, 0).await;

            deleter.enqueue(first).await.unwrap();
            deleter.enqueue(second.clone()).await.unwrap();

            // newest record wins: at most one entry per path.
            assert_eq!(deleter.entries, vec![second]);
        }

        #[tokio::test]
        async fn full_queue_rejects_new_paths_but_replaces_existing() {
            let tmp_a = temp_file(b"aaaa").await;
            let tmp_b = temp_file(b"bbbb").await;
            let clock = Clock::new(1000);
            let mut deleter = SingleThreadDeleter::new(DeleterArgs {
                now_fn: Arc::new(clock.now_fn()),
                queue_capacity: 1,
                snapshot_file: None,
            });
            let first = pending(tmp_a.file(), 1000, 0).await;
            deleter.enqueue(first.clone()).await.unwrap();

            // a new path is rejected at capacity; the queue is unchanged and
            // the rejected file stays on disk.
            let rejected = pending(tmp_b.file(), 1000, 0).await;
            let err = deleter.enqueue(rejected).await.unwrap_err();
            assert!(matches!(err, DeleteErr::QueueFullErr(_)));
            assert!(err.to_string().contains("capacity 1"));
            assert_eq!(deleter.entries, vec![first]);
            assert!(tmp_b.file().exists());

            // a same-path record still replaces at capacity (the net queue
            // length is unchanged).
            let replacement = pending(tmp_a.file(), 1200, 30).await;
            deleter.enqueue(replacement.clone()).await.unwrap();
            assert_eq!(deleter.entries, vec![replacement]);
        }
    }

    mod sweep {
        use super::*;

        #[tokio::test]
        async fn zero_delay_entry_is_deleted_on_first_sweep() {
            let tmp = temp_file(b"aaaa").await;
            let clock = Clock::new(1000);
            let mut deleter = deleter(&clock);
            deleter
                .enqueue(pending(tmp.file(), 1000, 0).await)
                .await
                .unwrap();

            deleter.sweep().await.unwrap();

            assert!(deleter.entries.is_empty());
            assert!(!tmp.file().exists());
        }

        #[tokio::test]
        async fn positive_delay_entry_waits_for_due_at() {
            let tmp = temp_file(b"aaaa").await;
            let clock = Clock::new(1000);
            let mut deleter = deleter(&clock);
            let record = pending(tmp.file(), 1000, 500).await;
            deleter.enqueue(record.clone()).await.unwrap();

            // not yet due: both 1000 and 1499 are before due_at (1500).
            deleter.sweep().await.unwrap();
            clock.advance(499);
            deleter.sweep().await.unwrap();
            assert_eq!(deleter.entries, vec![record]);
            assert!(tmp.file().exists());

            // due exactly at due_at.
            clock.advance(1);
            deleter.sweep().await.unwrap();
            assert!(deleter.entries.is_empty());
            assert!(!tmp.file().exists());
        }

        #[tokio::test]
        async fn size_changed_file_is_dropped_without_deleting() {
            let tmp = temp_file(b"aaaa").await;
            let clock = Clock::new(1000);
            let mut deleter = deleter(&clock);
            deleter
                .enqueue(pending(tmp.file(), 1000, 0).await)
                .await
                .unwrap();

            // the file grows after the record was taken.
            files::write_bytes(tmp.file(), b"aaaa-grown", WriteOptions::OVERWRITE_NONATOMIC)
                .await
                .unwrap();

            deleter.sweep().await.unwrap();

            assert!(deleter.entries.is_empty());
            assert!(tmp.file().exists());
        }

        #[tokio::test]
        async fn mtime_changed_file_is_dropped_without_deleting() {
            let tmp = temp_file(b"aaaa").await;
            let clock = Clock::new(1000);
            let mut deleter = deleter(&clock);
            let mut record = pending(tmp.file(), 1000, 0).await;
            // same size, different recorded mtime: the re-stat mismatches.
            record.mtime += chrono::Duration::seconds(1);
            deleter.enqueue(record).await.unwrap();

            deleter.sweep().await.unwrap();

            assert!(deleter.entries.is_empty());
            assert!(tmp.file().exists());
        }

        #[tokio::test]
        async fn missing_file_is_dropped_as_success() {
            let tmp = temp_file(b"aaaa").await;
            let clock = Clock::new(1000);
            let mut deleter = deleter(&clock);
            deleter
                .enqueue(pending(tmp.file(), 1000, 0).await)
                .await
                .unwrap();
            files::delete(tmp.file()).await.unwrap();

            deleter.sweep().await.unwrap();

            assert!(deleter.entries.is_empty());
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
            let record = PendingDelete {
                file: target.clone(),
                size: metadata.len(),
                mtime: DateTime::<Utc>::from(metadata.modified().unwrap()),
                digest: "sha256:unused".to_string(),
                eligible_at: DateTime::from_timestamp(1000, 0).unwrap(),
                delete_delay_secs: 0,
                upload_rule_id: "rule_1".to_string(),
                deployment_id: "dpl_1".to_string(),
            };
            deleter.enqueue(record.clone()).await.unwrap();

            deleter.sweep().await.unwrap();

            assert_eq!(deleter.entries, vec![record]);
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
            let record = PendingDelete {
                file: child,
                size: 4,
                mtime: DateTime::from_timestamp(900, 0).unwrap(),
                digest: "sha256:unused".to_string(),
                eligible_at: DateTime::from_timestamp(1000, 0).unwrap(),
                delete_delay_secs: 0,
                upload_rule_id: "rule_1".to_string(),
                deployment_id: "dpl_1".to_string(),
            };
            deleter.enqueue(record.clone()).await.unwrap();

            deleter.sweep().await.unwrap();

            assert_eq!(deleter.entries, vec![record]);
        }
    }

    mod persistence {
        use super::*;

        async fn snapshot_file(file: &File) -> DeleteQueueSnapshotFile {
            DeleteQueueSnapshotFile::new_with_default(file.clone(), DeleteQueueSnapshot::default())
                .await
                .unwrap()
        }

        #[tokio::test]
        async fn queue_survives_rebuild_from_snapshot() {
            let dir = dirs::temp("delete-snapshot").unwrap();
            let state_path = dir.file("delete_queue.json");
            let tmp = temp_file(b"aaaa").await;
            let clock = Clock::new(1000);
            let record = pending(tmp.file(), 1000, 500).await;

            let mut deleter = SingleThreadDeleter::new(DeleterArgs {
                now_fn: Arc::new(clock.now_fn()),
                snapshot_file: Some(snapshot_file(&state_path).await),
                ..DeleterArgs::default()
            });
            deleter.enqueue(record.clone()).await.unwrap();
            drop(deleter);

            // a rebuild from the same file re-seeds the pending entries.
            let mut rebuilt = SingleThreadDeleter::new(DeleterArgs {
                now_fn: Arc::new(clock.now_fn()),
                snapshot_file: Some(snapshot_file(&state_path).await),
                ..DeleterArgs::default()
            });
            assert_eq!(rebuilt.entries, vec![record]);

            // sweeping the now-due entry persists the drop too.
            clock.advance(500);
            rebuilt.sweep().await.unwrap();
            drop(rebuilt);

            let restored = SingleThreadDeleter::new(DeleterArgs {
                snapshot_file: Some(snapshot_file(&state_path).await),
                ..DeleterArgs::default()
            });
            assert!(restored.entries.is_empty());
            assert!(!tmp.file().exists());
        }

        #[tokio::test]
        async fn persist_failure_is_swallowed() {
            let dir = dirs::temp("delete-snapshot").unwrap();
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
            dirs::create(&crate::filesys::Dir::new(state_path.path().clone()))
                .await
                .unwrap();

            // the enqueue still succeeds; the persist failure is only logged.
            deleter
                .enqueue(pending(tmp.file(), 1000, 500).await)
                .await
                .unwrap();
            assert_eq!(deleter.entries.len(), 1);
        }
    }
}
