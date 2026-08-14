// standard crates
use std::collections::VecDeque;

// internal crates
use crate::data_uploads::{
    queue::QueueJob,
    retention::{errors::*, job::Job},
};
use crate::filesys::state_file::SingleThreadStateFile;
use crate::models::Patch;
use crate::trace;

// external crates
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use uuid::Uuid;

impl QueueJob for Job {
    type QueueFullErr = DeleteErr;

    const LABEL: &'static str = "delete";

    /// A retention job is due only once its TTL has elapsed.
    fn due_at(&self) -> DateTime<Utc> {
        Job::due_at(self)
    }

    fn file(&self) -> String {
        self.file.to_string()
    }

    fn queue_full_err(capacity: usize, file: String) -> DeleteErr {
        DeleteErr::QueueFullErr(QueueFullErr {
            capacity,
            file,
            trace: trace!(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueueEntry {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    pub job: Job,
    #[serde(default)]
    pub attempts: u32,
    #[serde(default)]
    pub next_attempt_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DeleteQueueSnapshot {
    pub entries: Vec<QueueEntry>,
}

impl Patch<DeleteQueueSnapshot> for DeleteQueueSnapshot {
    fn patch(&mut self, patch: DeleteQueueSnapshot) {
        *self = patch;
    }
}

/// The queue's persistence handle: an atomically-written JSON snapshot with an
/// in-memory cache.
pub type DeleteQueueSnapshotFile = SingleThreadStateFile<DeleteQueueSnapshot, DeleteQueueSnapshot>;

/// In-memory job queue with an optional snapshot. Every mutation (`enqueue`,
/// `remove`, `requeue`) persists as its last act, so there is exactly one
/// persist implementation and no caller can forget to call it. Selection
/// (`next_ready`, `count_ready`) never mutates and never persists.
pub struct Queue {
    entries: VecDeque<QueueEntry>,
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

    /// The number of queued entries, including one currently being swept: a
    /// selected entry stays queued until it resolves.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Push a new job at the tail under a fresh id. When the queue is full the
    /// enqueue is rejected with `DeleteErr::QueueFullErr` and nothing is
    /// persisted.
    pub async fn enqueue(&mut self, job: Job) -> Result<(), DeleteErr> {
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
        self.entries.push_back(QueueEntry {
            id: Uuid::new_v4(),
            job,
            attempts: 0,
            next_attempt_at: None,
        });
        self.persist().await;
        info!("delete: job enqueued; queue length {}", self.entries.len());
        Ok(())
    }

    pub fn next_ready(&self, now: DateTime<Utc>) -> Option<QueueEntry> {
        self.entries
            .iter()
            .find(|entry| Self::is_ready(entry, now))
            .cloned()
    }

    /// How many entries are due at `now`. This is the sweep's loop budget —
    /// one visit per due entry — not an idle accessor.
    pub fn count_ready(&self, now: DateTime<Utc>) -> usize {
        self.entries
            .iter()
            .filter(|entry| Self::is_ready(entry, now))
            .count()
    }

    fn is_ready(entry: &QueueEntry, now: DateTime<Utc>) -> bool {
        entry.job.due_at() <= now && entry.next_attempt_at.is_none_or(|at| at <= now)
    }

    /// Drop the entry with `id` and persist the shortened queue. This is the
    /// only point at which a job leaves durable storage, so call it only once
    /// the sweep has resolved the entry. Unknown ids are ignored.
    pub async fn remove(&mut self, id: Uuid) -> Option<QueueEntry> {
        if let Some(entry) = self.remove_impl(id) {
            self.persist().await;
            info!("delete: job removed; queue length {}", self.entries.len());
            return Some(entry);
        }
        None
    }

    fn remove_impl(&mut self, id: Uuid) -> Option<QueueEntry> {
        let idx = self.entries.iter().position(|entry| entry.id == id)?;
        self.entries.remove(idx)
    }

    /// Move `entry` to the tail, replacing whatever is queued under its id.
    /// This is a rotation of an entry the queue already holds, not an
    /// admission, so it is not capacity-gated and cannot change [`Self::len`].
    pub async fn requeue(&mut self, entry: QueueEntry) {
        self.remove_impl(entry.id);
        self.entries.push_back(entry);
        self.persist().await;
        info!("delete: job requeued; queue length {}", self.entries.len());
    }

    /// The sole writer to disk. Called by every mutator as its last act.
    async fn persist(&mut self) {
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
        self.entries.iter().map(|entry| entry.job.clone()).collect()
    }

    /// The queued entries, oldest enqueue first (test observability only).
    #[cfg(test)]
    pub(crate) fn queue_entries(&self) -> Vec<QueueEntry> {
        self.entries.iter().cloned().collect()
    }
}
