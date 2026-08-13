// internal crates
use crate::data_uploads::retention::{errors::*, job::Job};
use crate::filesys::state_file::SingleThreadStateFile;
use crate::models::Patch;
use crate::trace;

// external crates
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DeleteQueueSnapshot {
    pub entries: Vec<Job>,
}

impl Patch<DeleteQueueSnapshot> for DeleteQueueSnapshot {
    fn patch(&mut self, patch: DeleteQueueSnapshot) {
        *self = patch;
    }
}

/// The queue's persistence handle: an atomically-written JSON snapshot with an
/// in-memory cache. Mirrors the uploader's `QueueSnapshotFile`.
pub type DeleteQueueSnapshotFile = SingleThreadStateFile<DeleteQueueSnapshot, DeleteQueueSnapshot>;

/// Membership policy for the retention worker's jobs, mirroring the uploader's
/// `upload::queue::Queue`: at most one entry per file path (newest job wins), a
/// capacity bound that gates only new paths, and snapshot persistence on every
/// mutation. The queue decides *what is in the set*; the sweep's filesystem
/// mechanics live with the deleter.
pub struct Queue {
    entries: Vec<Job>,
    capacity: usize,
    snapshot_file: Option<DeleteQueueSnapshotFile>,
}

impl Queue {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: Vec::new(),
            capacity,
            snapshot_file: None,
        }
    }

    /// Seed the queue from `snapshot_file`'s persisted entries. The backlog is
    /// loaded in full even if it exceeds `capacity`; capacity only gates new
    /// enqueues, so an over-capacity backlog simply drains before the queue
    /// accepts more.
    pub fn from_snapshot(capacity: usize, snapshot_file: DeleteQueueSnapshotFile) -> Self {
        let entries = snapshot_file.read().entries.clone();
        Self {
            entries,
            capacity,
            snapshot_file: Some(snapshot_file),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Record `job` as the queue's newest knowledge of its path: any existing
    /// entry for the same file is replaced (newest job wins), so the queue
    /// holds at most one entry per path. At capacity a NEW path is rejected
    /// with [`DeleteErr::QueueFullErr`] — the file simply stays on disk, the
    /// safe direction — while a same-path replacement always succeeds (it
    /// never grows the queue), even on a snapshot-seeded over-capacity
    /// backlog.
    pub async fn enqueue(&mut self, job: Job) -> Result<(), DeleteErr> {
        let replaces_existing = self.entries.iter().any(|entry| entry.file == job.file);
        if !replaces_existing && self.entries.len() >= self.capacity {
            warn!(
                "delete: queue is full (capacity {}); rejecting job for file {}",
                self.capacity, job.file
            );
            return Err(DeleteErr::QueueFullErr(QueueFullErr {
                capacity: self.capacity,
                file: job.file.to_string(),
                trace: trace!(),
            }));
        }
        self.entries.retain(|entry| entry.file != job.file);
        self.entries.push(job);
        self.persist_snapshot().await;
        info!("delete: job enqueued; queue length {}", self.entries.len());
        Ok(())
    }

    /// Take every job out of the queue for a sweep pass. The caller must hand
    /// the survivors back via [`restore`](Self::restore) — the queue is empty
    /// (and its snapshot intentionally untouched) in between.
    pub fn drain(&mut self) -> Vec<Job> {
        std::mem::take(&mut self.entries)
    }

    /// Put a sweep pass's surviving jobs back, persisting iff the pass dropped
    /// any (`before` is the count handed out by [`drain`](Self::drain)).
    pub async fn restore(&mut self, kept: Vec<Job>, before: usize) {
        self.entries = kept;
        if self.entries.len() != before {
            self.persist_snapshot().await;
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

    /// The queued jobs, oldest enqueue first (test observability only).
    #[cfg(test)]
    pub(crate) fn entries(&self) -> &[Job] {
        &self.entries
    }
}

#[cfg(test)]
mod tests {
    // internal crates
    use super::{DeleteQueueSnapshot, DeleteQueueSnapshotFile, Queue};
    use crate::data_uploads::retention::errors::DeleteErr;
    use crate::data_uploads::retention::job::Job;
    use crate::filesys::{dirs, files, File, PathExt, WriteOptions};

    // external crates
    use chrono::{DateTime, Utc};

    const DEFAULT_CAPACITY: usize = 4096;

    /// A real on-disk temp file holding `contents`; the returned guard deletes
    /// it on drop.
    async fn temp_file(contents: &[u8]) -> files::TempFile {
        let tmp = files::temp("delete-queue-test").unwrap();
        files::write_bytes(tmp.file(), contents, WriteOptions::OVERWRITE_NONATOMIC)
            .await
            .unwrap();
        tmp
    }

    /// A `Job` for `file` whose size/mtime/digest reflect the file's current
    /// on-disk state.
    async fn make_job(file: &File, eligible_secs: i64, ttl_secs: u64) -> Job {
        Job {
            file: file.clone(),
            size: files::size(file).await.unwrap(),
            mtime: DateTime::<Utc>::from(files::last_modified(file).await.unwrap()),
            digest: files::hash(file).await.unwrap(),
            eligible_at: DateTime::from_timestamp(eligible_secs, 0).unwrap(),
            ttl_secs,
            file_rule_id: "file_rule_1".to_string(),
            deployment_id: "dpl_1".to_string(),
        }
    }

    /// A persistence handle for the snapshot at `file`.
    async fn snapshot_file(file: &File) -> DeleteQueueSnapshotFile {
        DeleteQueueSnapshotFile::new_with_default(file.clone(), DeleteQueueSnapshot::default())
            .await
            .unwrap()
    }

    mod enqueue {
        use super::*;

        #[tokio::test]
        async fn same_path_enqueue_replaces_older_job() {
            let tmp = temp_file(b"aaaa").await;
            let mut queue = Queue::new(DEFAULT_CAPACITY);
            let first = make_job(tmp.file(), 1000, 100).await;
            let second = make_job(tmp.file(), 1200, 0).await;

            queue.enqueue(first).await.unwrap();
            queue.enqueue(second.clone()).await.unwrap();

            // newest job wins: at most one entry per path.
            assert_eq!(queue.entries(), [second]);
        }

        #[tokio::test]
        async fn full_queue_rejects_new_paths_but_replaces_existing() {
            let tmp_a = temp_file(b"aaaa").await;
            let tmp_b = temp_file(b"bbbb").await;
            let mut queue = Queue::new(1);
            let first = make_job(tmp_a.file(), 1000, 0).await;
            queue.enqueue(first.clone()).await.unwrap();

            // a new path is rejected at capacity; the queue is unchanged and
            // the rejected file stays on disk.
            let rejected = make_job(tmp_b.file(), 1000, 0).await;
            let err = queue.enqueue(rejected).await.unwrap_err();
            assert!(matches!(err, DeleteErr::QueueFullErr(_)));
            assert!(err.to_string().contains("capacity 1"));
            assert_eq!(queue.entries(), [first.clone()]);
            assert!(tmp_b.file().exists());

            // a same-path job still replaces at capacity (the net queue length
            // is unchanged).
            let replacement = make_job(tmp_a.file(), 1200, 30).await;
            queue.enqueue(replacement.clone()).await.unwrap();
            assert_eq!(queue.entries(), [replacement]);
        }

        // A snapshot-seeded backlog may exceed `capacity` (capacity only gates
        // new enqueues). A same-path job must still replace on such a queue
        // rather than being rejected after its older job was dropped.
        #[tokio::test]
        async fn over_capacity_backlog_still_replaces_existing() {
            let dir = dirs::temp("delete-over-capacity").unwrap();
            let state_path = dir.file("delete_queue.json");
            let tmp_a = temp_file(b"aaaa").await;
            let tmp_b = temp_file(b"bbbb").await;
            let first_a = make_job(tmp_a.file(), 1000, 100).await;
            let first_b = make_job(tmp_b.file(), 1000, 100).await;

            // write a two-entry snapshot, then rebuild with capacity 1: the
            // seeded backlog exceeds capacity by design.
            {
                let mut seeder =
                    Queue::from_snapshot(DEFAULT_CAPACITY, snapshot_file(&state_path).await);
                seeder.enqueue(first_a.clone()).await.unwrap();
                seeder.enqueue(first_b.clone()).await.unwrap();
            }
            let mut queue = Queue::from_snapshot(1, snapshot_file(&state_path).await);
            assert_eq!(queue.entries(), [first_a.clone(), first_b.clone()]);

            // a same-path job replaces its entry (newest job wins); nothing is
            // silently lost from the over-capacity queue.
            let replacement = make_job(tmp_a.file(), 1200, 0).await;
            queue.enqueue(replacement.clone()).await.unwrap();
            assert_eq!(queue.entries(), [first_b.clone(), replacement]);

            // a genuinely new path is still rejected, leaving the queue intact.
            let tmp_c = temp_file(b"cccc").await;
            let err = queue
                .enqueue(make_job(tmp_c.file(), 1000, 0).await)
                .await
                .unwrap_err();
            assert!(matches!(err, DeleteErr::QueueFullErr(_)));
            assert_eq!(queue.len(), 2);
        }
    }

    mod persistence {
        use super::*;

        #[tokio::test]
        async fn queue_survives_rebuild_from_snapshot() {
            let dir = dirs::temp("delete-snapshot").unwrap();
            let state_path = dir.file("delete_queue.json");
            let tmp = temp_file(b"aaaa").await;
            let job = make_job(tmp.file(), 1000, 500).await;

            let mut queue =
                Queue::from_snapshot(DEFAULT_CAPACITY, snapshot_file(&state_path).await);
            queue.enqueue(job.clone()).await.unwrap();
            drop(queue);

            // a rebuild from the same file re-seeds the queued jobs.
            let rebuilt = Queue::from_snapshot(DEFAULT_CAPACITY, snapshot_file(&state_path).await);
            assert_eq!(rebuilt.entries(), [job]);
        }

        #[tokio::test]
        async fn restore_persists_only_when_jobs_were_dropped() {
            let dir = dirs::temp("delete-snapshot").unwrap();
            let state_path = dir.file("delete_queue.json");
            let tmp_a = temp_file(b"aaaa").await;
            let tmp_b = temp_file(b"bbbb").await;
            let mut queue =
                Queue::from_snapshot(DEFAULT_CAPACITY, snapshot_file(&state_path).await);
            let job_a = make_job(tmp_a.file(), 1000, 0).await;
            let job_b = make_job(tmp_b.file(), 1000, 500).await;
            queue.enqueue(job_a).await.unwrap();
            queue.enqueue(job_b.clone()).await.unwrap();

            // a pass that drops a job persists the survivors.
            let taken = queue.drain();
            let before = taken.len();
            let kept: Vec<_> = taken.into_iter().filter(|j| j.ttl_secs > 0).collect();
            queue.restore(kept, before).await;
            drop(queue);

            let restored = Queue::from_snapshot(DEFAULT_CAPACITY, snapshot_file(&state_path).await);
            assert_eq!(restored.entries(), [job_b]);
        }

        #[tokio::test]
        async fn persist_failure_is_swallowed() {
            let dir = dirs::temp("delete-snapshot").unwrap();
            let state_path = dir.file("delete_queue.json");
            let tmp = temp_file(b"aaaa").await;
            let mut queue =
                Queue::from_snapshot(DEFAULT_CAPACITY, snapshot_file(&state_path).await);

            // make the snapshot path unwritable: a DIRECTORY now sits there.
            files::delete(&state_path).await.unwrap();
            dirs::create(&crate::filesys::Dir::new(state_path.path().clone()))
                .await
                .unwrap();

            // the enqueue still succeeds; the persist failure is only logged.
            queue
                .enqueue(make_job(tmp.file(), 1000, 500).await)
                .await
                .unwrap();
            assert_eq!(queue.len(), 1);
        }
    }
}
