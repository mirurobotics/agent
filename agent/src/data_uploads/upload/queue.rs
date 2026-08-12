// standard crates
use std::collections::VecDeque;

// internal crates
use crate::data_uploads::upload::{
    errors::{QueueFullErr, UploadErr},
    job::Job,
};
use crate::filesys::state_file::SingleThreadStateFile;
use crate::models::Patch;
use crate::trace;

// external crates
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueueEntry {
    pub job: Job,
    pub attempts: u32,
    /// Earliest instant this entry is eligible to be popped; `None` means
    /// "eligible now".
    #[serde(default)]
    pub next_attempt_at: Option<DateTime<Utc>>,
}

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
    pub fn from_snapshot(capacity: usize, snapshot_file: QueueSnapshotFile) -> Self {
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

    /// Push a new job at the tail. When the queue is full the enqueue is
    /// rejected with `UploadErr::QueueFullErr`.
    pub async fn enqueue(&mut self, job: Job) -> Result<(), UploadErr> {
        self.verify_capacity(&job).await?;
        self.jobs.push_back(QueueEntry {
            job,
            attempts: 0,
            next_attempt_at: None,
        });
        self.persist().await;
        info!("upload: job enqueued; queue length {}", self.jobs.len());
        Ok(())
    }

    /// Push a previously popped job back at the tail, preserving its attempt
    pub async fn requeue(&mut self, entry: QueueEntry) {
        self.jobs.push_back(entry);
        self.persist().await;
        info!("upload: job requeued; queue length {}", self.jobs.len());
    }

    /// Remove and return the first entry whose `next_attempt_at` is `None` or
    /// `<= now`, preserving the order of the remaining entries. Returns `None`
    /// (without persisting) when no entry is eligible.
    pub async fn pop_ready(&mut self, now: DateTime<Utc>) -> Option<QueueEntry> {
        let idx = self
            .jobs
            .iter()
            .position(|entry| entry.next_attempt_at.is_none_or(|at| at <= now))?;
        let entry = self.jobs.remove(idx);
        if entry.is_some() {
            self.persist().await;
            info!("upload: job dequeued; queue length {}", self.jobs.len());
        }
        entry
    }

    /// The minimum effective deadline over all entries, where a `None`
    /// deadline counts as `DateTime::<Utc>::MIN_UTC`. Returns `None` only when
    /// the queue is empty. The worker uses it to size an idle sleep.
    pub fn earliest_next_attempt(&self) -> Option<DateTime<Utc>> {
        self.jobs
            .iter()
            .map(|entry| entry.next_attempt_at.unwrap_or(DateTime::<Utc>::MIN_UTC))
            .min()
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
