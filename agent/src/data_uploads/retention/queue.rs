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
/// in-memory cache.
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

    /// Call after a mutation that should survive a restart.
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
    use chrono::DateTime;

    const DEFAULT_CAPACITY: usize = 4096;

    /// A deterministic `Job` — fixed size/digest/mtime derived from `name`, so
    /// whole-struct equality holds across snapshot reloads. The queue never
    /// touches the file's contents, so no file exists at the path.
    fn make_job(name: &str, observed_secs: i64, ttl_secs: u64) -> Job {
        let observed_at = DateTime::from_timestamp(observed_secs, 0).unwrap();
        Job {
            file: File::new(format!("/data/{name}")),
            size: 42,
            digest: format!("sha256:{name}"),
            mtime: DateTime::from_timestamp(900, 0).unwrap(),
            first_observed_at: observed_at,
            last_observed_at: observed_at,
            ttl_secs,
            file_rule_id: "file_rule_1".to_string(),
            deployment_id: "dpl_1".to_string(),
        }
    }

    /// A fresh snapshot handle over `path`. Reopening the same path returns a
    /// handle whose in-memory cache reflects what was previously persisted.
    async fn open(path: &File) -> DeleteQueueSnapshotFile {
        DeleteQueueSnapshotFile::new_with_default(path.clone(), DeleteQueueSnapshot::default())
            .await
            .unwrap()
    }

    mod from_snapshot {
        use super::*;

        #[tokio::test]
        async fn empty_snapshot_loads_empty_queue() {
            let dir = dirs::temp("delete-queue-test").unwrap();
            let path = dir.file("delete_queue.json");

            let queue = Queue::from_snapshot(8, open(&path).await);

            assert!(queue.is_empty());
        }

        // Pins the persisted wire format field-by-field
        #[tokio::test]
        async fn raw_json_snapshot_loads() {
            let dir = dirs::temp("delete-queue-test").unwrap();
            let path = dir.file("delete_queue.json");
            let raw = concat!(
                r#"{"entries":[{"file":"/data/a.log","size":42,"#,
                r#""digest":"sha256:a.log","mtime":"1970-01-01T00:15:00Z","#,
                r#""first_observed_at":"1970-01-01T00:16:40Z","#,
                r#""last_observed_at":"1970-01-01T00:16:40Z","ttl_secs":500,"#,
                r#""file_rule_id":"file_rule_1","deployment_id":"dpl_1"}]}"#,
            );
            files::write_string(&path, raw, WriteOptions::OVERWRITE_ATOMIC)
                .await
                .unwrap();

            let queue = Queue::from_snapshot(8, open(&path).await);

            assert_eq!(queue.entries(), [make_job("a.log", 1000, 500)]);
        }
    }

    mod enqueue {
        use super::*;

        #[test]
        fn appends_in_fifo_order() {
            let mut queue = Queue::new(DEFAULT_CAPACITY);
            let job_a = make_job("a.log", 1000, 0);
            let job_b = make_job("b.log", 1100, 60);
            let job_c = make_job("c.log", 1200, 0);

            queue.enqueue(job_a.clone()).unwrap();
            queue.enqueue(job_b.clone()).unwrap();
            queue.enqueue(job_c.clone()).unwrap();

            assert_eq!(queue.entries(), [job_a, job_b, job_c]);
        }

        #[test]
        fn duplicate_same_path_jobs_are_both_queued() {
            let mut queue = Queue::new(DEFAULT_CAPACITY);
            let first = make_job("a.log", 1000, 100);
            let second = make_job("a.log", 1200, 0);

            queue.enqueue(first.clone()).unwrap();
            queue.enqueue(second.clone()).unwrap();

            assert_eq!(queue.entries(), [first, second]);
        }

        #[test]
        fn full_queue_returns_queue_full_err() {
            let mut queue = Queue::new(1);
            let first = make_job("a.log", 1000, 0);
            queue.enqueue(first.clone()).unwrap();

            // a new path is rejected at capacity and the queue is unchanged.
            let err = queue.enqueue(make_job("b.log", 1000, 0)).unwrap_err();
            let DeleteErr::QueueFullErr(err) = err else {
                panic!("expected QueueFullErr, got: {err:?}");
            };
            assert_eq!(err.capacity, 1);
            assert_eq!(err.file, "/data/b.log");
            assert_eq!(queue.entries().as_slice(), std::slice::from_ref(&first));

            // same-path is not a bypass: a duplicate also grows the queue, so
            // it is rejected too.
            let err = queue.enqueue(make_job("a.log", 1200, 30)).unwrap_err();
            assert!(matches!(err, DeleteErr::QueueFullErr(_)));
            assert_eq!(queue.entries(), [first]);
        }

        #[tokio::test]
        async fn does_not_persist() {
            let dir = dirs::temp("delete-queue-test").unwrap();
            let path = dir.file("delete_queue.json");
            let mut queue = Queue::from_snapshot(DEFAULT_CAPACITY, open(&path).await);

            queue.enqueue(make_job("a.log", 1000, 500)).unwrap();
            drop(queue);

            let rebuilt = Queue::from_snapshot(DEFAULT_CAPACITY, open(&path).await);
            assert!(rebuilt.is_empty());
        }
    }

    mod pop_front {
        use super::*;

        #[test]
        fn returns_jobs_in_fifo_order() {
            let mut queue = Queue::new(DEFAULT_CAPACITY);
            let job_a = make_job("a.log", 1000, 0);
            let job_b = make_job("b.log", 1100, 60);
            queue.enqueue(job_a.clone()).unwrap();
            queue.enqueue(job_b.clone()).unwrap();

            assert_eq!(queue.pop_front(), Some(job_a));
            assert_eq!(queue.len(), 1);
            assert_eq!(queue.pop_front(), Some(job_b));
            assert_eq!(queue.pop_front(), None);
            assert!(queue.is_empty());
        }

        #[tokio::test]
        async fn does_not_persist() {
            let dir = dirs::temp("delete-queue-test").unwrap();
            let path = dir.file("delete_queue.json");
            let mut queue = Queue::from_snapshot(DEFAULT_CAPACITY, open(&path).await);
            let job_a = make_job("a.log", 1000, 0);
            let job_b = make_job("b.log", 1000, 500);
            queue.enqueue(job_a.clone()).unwrap();
            queue.enqueue(job_b.clone()).unwrap();
            queue.persist().await;

            // in-flight: the job is out of memory but still on disk, so a
            // crash before persist reloads both.
            assert!(queue.pop_front().is_some());
            drop(queue);

            let restored = Queue::from_snapshot(DEFAULT_CAPACITY, open(&path).await);
            assert_eq!(restored.entries(), [job_a, job_b]);
        }
    }

    mod requeue {
        use super::*;

        #[test]
        fn appends_at_tail() {
            let mut queue = Queue::new(DEFAULT_CAPACITY);
            let job_a = make_job("a.log", 1000, 0);
            let job_b = make_job("b.log", 1100, 60);
            queue.enqueue(job_a.clone()).unwrap();
            queue.enqueue(job_b.clone()).unwrap();

            // a popped-and-requeued job goes behind the remaining entries, so
            // a sweep visiting the queue head-to-tail sees each job once.
            let popped = queue.pop_front().unwrap();
            queue.requeue(popped.clone());

            assert_eq!(queue.entries(), [job_b, popped]);
        }

        // Requeue must never be gated by capacity: the sweep pops each job
        // into ownership before deciding, and a kept job on an at- or
        // over-capacity queue has to go back.
        #[test]
        fn bypasses_capacity() {
            let mut queue = Queue::new(1);
            let job_a = make_job("a.log", 1000, 0);
            queue.enqueue(job_a.clone()).unwrap();

            let popped = queue.pop_front().unwrap();
            queue.enqueue(make_job("b.log", 1000, 0)).unwrap();
            queue.requeue(popped.clone());

            assert_eq!(queue.entries(), [make_job("b.log", 1000, 0), popped]);
            assert_eq!(queue.len(), 2);
        }
    }

    mod persist {
        use super::*;

        #[tokio::test]
        async fn round_trips_the_queue() {
            let dir = dirs::temp("delete-queue-test").unwrap();
            let path = dir.file("delete_queue.json");
            let job = make_job("a.log", 1000, 500);

            let mut queue = Queue::from_snapshot(DEFAULT_CAPACITY, open(&path).await);
            queue.enqueue(job.clone()).unwrap();
            queue.persist().await;
            drop(queue);

            let rebuilt = Queue::from_snapshot(DEFAULT_CAPACITY, open(&path).await);
            assert_eq!(rebuilt.entries(), [job]);
        }

        #[tokio::test]
        async fn after_pop_drops_the_front_job() {
            let dir = dirs::temp("delete-queue-test").unwrap();
            let path = dir.file("delete_queue.json");
            let mut queue = Queue::from_snapshot(DEFAULT_CAPACITY, open(&path).await);
            let job_b = make_job("b.log", 1000, 500);
            queue.enqueue(make_job("a.log", 1000, 0)).unwrap();
            queue.enqueue(job_b.clone()).unwrap();
            queue.persist().await;

            assert!(queue.pop_front().is_some());
            queue.persist().await;
            drop(queue);

            let restored = Queue::from_snapshot(DEFAULT_CAPACITY, open(&path).await);
            assert_eq!(restored.entries(), [job_b]);
        }

        #[tokio::test]
        async fn failure_is_swallowed() {
            let dir = dirs::temp("delete-queue-test").unwrap();
            let path = dir.file("delete_queue.json");
            let mut queue = Queue::from_snapshot(DEFAULT_CAPACITY, open(&path).await);

            // make the snapshot path unwritable: a DIRECTORY now sits there.
            files::delete(&path).await.unwrap();
            dirs::create(&crate::filesys::Dir::new(path.path().clone()))
                .await
                .unwrap();

            queue.enqueue(make_job("a.log", 1000, 500)).unwrap();
            // the persist failure is only logged; memory still holds the job.
            queue.persist().await;
            assert_eq!(queue.len(), 1);
        }
    }
}
