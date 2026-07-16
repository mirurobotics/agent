// standard crates
use std::collections::VecDeque;

// internal crates
use crate::filesys::state_file::SingleThreadStateFile;
use crate::models::Patch;
use crate::trace;
use crate::upload::{
    errors::{QueueFullErr, UploadErr},
    job::Job,
};

// external crates
use serde::{Deserialize, Serialize};
use tracing::warn;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueueEntry {
    pub job: Job,
    pub attempts: u32,
}

/// The on-disk form of the queue: the ordered backlog of jobs that have not yet
/// finished uploading. Persisted after every mutation so a power failure or
/// restart resumes the backlog instead of dropping it. The in-flight job (one
/// popped for upload but not yet confirmed) is deliberately absent — an
/// interrupted transfer is re-driven via scanner re-observation plus backend
/// digest dedup (see the cancel-safety contract on [`super::UploadExecutor`]).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct QueueSnapshot {
    pub entries: Vec<QueueEntry>,
}

impl Patch<QueueSnapshot> for QueueSnapshot {
    fn patch(&mut self, patch: QueueSnapshot) {
        *self = patch;
    }
}

/// The queue's persistence handle: an atomically-written JSON snapshot with an
/// in-memory cache. Mirrors the scanner's `ScanSnapshotFile`.
pub type QueueSnapshotFile = SingleThreadStateFile<QueueSnapshot, QueueSnapshot>;

/// A FIFO queue of upload jobs with rejection (via [`QueueFullErr`]) when full.
///
/// When constructed with [`Queue::with_snapshot`], the backlog is loaded from
/// disk on startup and written back after every mutation, so queued uploads
/// survive an agent restart or power failure. Constructed with [`Queue::new`]
/// it is purely in-memory.
pub struct Queue {
    jobs: VecDeque<QueueEntry>,
    capacity: usize,
    snapshot_file: Option<QueueSnapshotFile>,
}

impl Queue {
    /// An in-memory-only queue with no disk persistence.
    pub fn new(capacity: usize) -> Self {
        Self {
            jobs: VecDeque::new(),
            capacity,
            snapshot_file: None,
        }
    }

    /// A disk-backed queue: `snapshot_file`'s persisted entries seed the initial
    /// backlog, and every subsequent mutation is written back. Persisted entries
    /// are loaded in full even if they exceed `capacity`; capacity only gates new
    /// enqueues, so an over-capacity backlog simply drains before it accepts more.
    pub fn with_snapshot(capacity: usize, snapshot_file: QueueSnapshotFile) -> Self {
        let jobs = snapshot_file.read().entries.iter().cloned().collect();
        Self {
            jobs,
            capacity,
            snapshot_file: Some(snapshot_file),
        }
    }

    pub fn len(&self) -> usize {
        self.jobs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    /// Push a new job at the tail, then persist. When full, the enqueue is
    /// rejected with `UploadErr::QueueFullErr` and nothing is persisted.
    pub async fn enqueue(&mut self, job: Job) -> Result<(), UploadErr> {
        self.verify_capacity(&job).await?;
        self.jobs.push_back(QueueEntry { job, attempts: 0 });
        self.persist().await;
        Ok(())
    }

    /// Push a previously popped job back at the tail, preserving its attempt
    /// count, then persist.
    pub async fn requeue(&mut self, entry: QueueEntry) -> Result<(), UploadErr> {
        self.verify_capacity(&entry.job).await?;
        self.jobs.push_back(entry);
        self.persist().await;
        Ok(())
    }

    /// Pop the front job for upload. Persists the shorter backlog when a job is
    /// removed; the popped (now in-flight) job is intentionally no longer on
    /// disk (see [`QueueSnapshot`]).
    pub async fn pop_front(&mut self) -> Option<QueueEntry> {
        let entry = self.jobs.pop_front();
        if entry.is_some() {
            self.persist().await;
        }
        entry
    }

    async fn verify_capacity(&mut self, job: &Job) -> Result<(), UploadErr> {
        if self.jobs.len() >= self.capacity {
            warn!(
                "upload queue is full (capacity {}); rejecting job for file {}",
                self.capacity, job.file
            );
            return Err(UploadErr::QueueFullErr(QueueFullErr {
                capacity: self.capacity,
                file: job.file.to_string(),
                trace: trace!(),
            }));
        }
        Ok(())
    }

    /// Write the current backlog to disk. A no-op for an in-memory queue.
    /// Persistence failures are logged and swallowed: a durable-storage hiccup
    /// must not stall or drop live uploads (mirrors the scanner's snapshotting).
    async fn persist(&mut self) {
        let Some(snapshot_file) = self.snapshot_file.as_mut() else {
            return;
        };
        let snapshot = QueueSnapshot {
            entries: self.jobs.iter().cloned().collect(),
        };
        if let Err(err) = snapshot_file.patch(snapshot).await {
            warn!("upload: failed to persist upload queue: {err}");
        }
    }
}
