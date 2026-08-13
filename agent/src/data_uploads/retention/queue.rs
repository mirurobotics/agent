// standard crates
use std::collections::VecDeque;

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

/// In-memory job queue with an optional snapshot. Mutations (`enqueue`,
/// `pop_front`, `requeue`) only touch memory; [`persist`] is the sole
/// writer to disk.
pub struct Queue {
    entries: VecDeque<Job>,
    capacity: usize,
    snapshot_file: Option<DeleteQueueSnapshotFile>,
}

impl Queue {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            capacity,
            snapshot_file: None,
        }
    }

    /// Seed the queue from `snapshot_file`'s persisted entries. The backlog is loaded
    /// in full even if it exceeds `capacity`; capacity only gates new enqueues, so an
    /// over-capacity backlog simply drains before the queue accepts more.
    pub fn from_snapshot(capacity: usize, snapshot_file: DeleteQueueSnapshotFile) -> Self {
        let entries = snapshot_file.read().entries.iter().cloned().collect();
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

    pub fn enqueue(&mut self, job: Job) -> Result<(), DeleteErr> {
        if self.entries.len() >= self.capacity {
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
        self.entries.push_back(job);
        info!("delete: job enqueued; queue length {}", self.entries.len());
        Ok(())
    }

    pub fn pop_front(&mut self) -> Option<Job> {
        self.entries.pop_front()
    }

    /// Push a previously popped job back at the tail.
    pub fn requeue(&mut self, job: Job) {
        self.entries.push_back(job);
    }

    /// Write the in-memory queue to the snapshot. The only function that
    /// touches disk; call it after a mutation that should survive a restart.
    pub async fn persist(&mut self) {
        let Some(snapshot_file) = self.snapshot_file.as_mut() else {
            return;
        };
        let snapshot = DeleteQueueSnapshot {
            entries: self.entries.iter().cloned().collect(),
        };
        if let Err(err) = snapshot_file.patch(snapshot).await {
            warn!("delete: failed to persist delete queue: {err}");
        }
    }

    /// The queued jobs, oldest enqueue first (test observability only).
    #[cfg(test)]
    pub(crate) fn entries(&self) -> Vec<Job> {
        self.entries.iter().cloned().collect()
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

    /// A persistence handle for the snapshot at `file`.
    async fn snapshot_file(file: &File) -> DeleteQueueSnapshotFile {
        DeleteQueueSnapshotFile::new_with_default(file.clone(), DeleteQueueSnapshot::default())
            .await
            .unwrap()
    }

    mod enqueue {
        use super::*;

        #[tokio::test]
        async fn same_path_enqueue_appends() {
            let tmp = temp_file(b"aaaa").await;
            let mut queue = Queue::new(DEFAULT_CAPACITY);
            let first = make_job(tmp.file(), 1000, 100).await;
            let second = make_job(tmp.file(), 1200, 0).await;

            queue.enqueue(first.clone()).unwrap();
            queue.enqueue(second.clone()).unwrap();

            assert_eq!(queue.entries(), [first, second]);
        }

        #[tokio::test]
        async fn full_queue_returns_queue_full_err() {
            let tmp_a = temp_file(b"aaaa").await;
            let tmp_b = temp_file(b"bbbb").await;
            let mut queue = Queue::new(1);
            let first = make_job(tmp_a.file(), 1000, 0).await;
            queue.enqueue(first.clone()).unwrap();

            // a new path is rejected at capacity; the queue is unchanged and
            // the rejected file stays on disk.
            let rejected = make_job(tmp_b.file(), 1000, 0).await;
            let err = queue.enqueue(rejected).unwrap_err();
            assert!(matches!(err, DeleteErr::QueueFullErr(_)));
            assert!(err.to_string().contains("capacity 1"));
            assert_eq!(queue.entries(), [first.clone()]);
            assert!(tmp_b.file().exists());

            // same-path is not a bypass: a second job for `tmp_a` also grows
            // the queue, so it is rejected too.
            let err = queue
                .enqueue(make_job(tmp_a.file(), 1200, 30).await)
                .unwrap_err();
            assert!(matches!(err, DeleteErr::QueueFullErr(_)));
            assert_eq!(queue.entries(), [first]);
        }

        // A snapshot-seeded backlog may exceed `capacity` (capacity only gates
        // new enqueues). Same-path is not a bypass on such a queue either.
        #[tokio::test]
        async fn over_capacity_backlog_rejects_new_enqueues() {
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
                seeder.enqueue(first_a.clone()).unwrap();
                seeder.enqueue(first_b.clone()).unwrap();
                seeder.persist().await;
            }
            let mut queue = Queue::from_snapshot(1, snapshot_file(&state_path).await);
            assert_eq!(queue.entries(), [first_a.clone(), first_b.clone()]);

            let err = queue
                .enqueue(make_job(tmp_a.file(), 1200, 0).await)
                .unwrap_err();
            assert!(matches!(err, DeleteErr::QueueFullErr(_)));
            assert_eq!(queue.entries(), [first_a.clone(), first_b.clone()]);

            let tmp_c = temp_file(b"cccc").await;
            let err = queue
                .enqueue(make_job(tmp_c.file(), 1000, 0).await)
                .unwrap_err();
            assert!(matches!(err, DeleteErr::QueueFullErr(_)));
            assert_eq!(queue.len(), 2);
        }
    }

    mod persistence {
        use super::*;

        #[tokio::test]
        async fn enqueue_does_not_persist() {
            let dir = dirs::temp("delete-snapshot").unwrap();
            let state_path = dir.file("delete_queue.json");
            let tmp = temp_file(b"aaaa").await;
            let mut queue =
                Queue::from_snapshot(DEFAULT_CAPACITY, snapshot_file(&state_path).await);
            queue
                .enqueue(make_job(tmp.file(), 1000, 500).await)
                .unwrap();
            drop(queue);

            let rebuilt = Queue::from_snapshot(DEFAULT_CAPACITY, snapshot_file(&state_path).await);
            assert!(rebuilt.is_empty());
        }

        #[tokio::test]
        async fn queue_survives_rebuild_from_snapshot() {
            let dir = dirs::temp("delete-snapshot").unwrap();
            let state_path = dir.file("delete_queue.json");
            let tmp = temp_file(b"aaaa").await;
            let job = make_job(tmp.file(), 1000, 500).await;

            let mut queue =
                Queue::from_snapshot(DEFAULT_CAPACITY, snapshot_file(&state_path).await);
            queue.enqueue(job.clone()).unwrap();
            queue.persist().await;
            drop(queue);

            // a rebuild from the same file re-seeds the queued jobs.
            let rebuilt = Queue::from_snapshot(DEFAULT_CAPACITY, snapshot_file(&state_path).await);
            assert_eq!(rebuilt.entries(), [job]);
        }

        #[tokio::test]
        async fn pop_front_does_not_persist() {
            let dir = dirs::temp("delete-snapshot").unwrap();
            let state_path = dir.file("delete_queue.json");
            let tmp_a = temp_file(b"aaaa").await;
            let tmp_b = temp_file(b"bbbb").await;
            let mut queue =
                Queue::from_snapshot(DEFAULT_CAPACITY, snapshot_file(&state_path).await);
            let job_a = make_job(tmp_a.file(), 1000, 0).await;
            let job_b = make_job(tmp_b.file(), 1000, 500).await;
            queue.enqueue(job_a.clone()).unwrap();
            queue.enqueue(job_b.clone()).unwrap();
            queue.persist().await;

            // in-flight: the job is out of memory but still on disk, so a
            // crash before persist reloads both.
            assert!(queue.pop_front().is_some());
            drop(queue);

            let restored = Queue::from_snapshot(DEFAULT_CAPACITY, snapshot_file(&state_path).await);
            assert_eq!(restored.entries(), [job_a, job_b]);
        }

        #[tokio::test]
        async fn persist_after_pop_drops_the_front_job() {
            let dir = dirs::temp("delete-snapshot").unwrap();
            let state_path = dir.file("delete_queue.json");
            let tmp_a = temp_file(b"aaaa").await;
            let tmp_b = temp_file(b"bbbb").await;
            let mut queue =
                Queue::from_snapshot(DEFAULT_CAPACITY, snapshot_file(&state_path).await);
            let job_a = make_job(tmp_a.file(), 1000, 0).await;
            let job_b = make_job(tmp_b.file(), 1000, 500).await;
            queue.enqueue(job_a).unwrap();
            queue.enqueue(job_b.clone()).unwrap();
            queue.persist().await;

            assert!(queue.pop_front().is_some());
            queue.persist().await;
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

            queue
                .enqueue(make_job(tmp.file(), 1000, 500).await)
                .unwrap();
            // the persist failure is only logged; memory still holds the job.
            queue.persist().await;
            assert_eq!(queue.len(), 1);
        }
    }
}
