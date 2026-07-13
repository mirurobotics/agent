// standard crates
use std::collections::VecDeque;
use std::time::SystemTime;

// internal crates
use crate::filesys::{files, FileSysErr};
use crate::trace;
use crate::upload::{
    errors::{QueueFullErr, UploadErr},
    job::{DedupKey, Job},
};

// external crates
use chrono::{DateTime, Utc};
use tracing::{info, warn};

/// The result of a successful enqueue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Enqueued,
    /// A job with the same dedup key is already pending; the enqueue was a
    /// no-op.
    Duplicate,
}

#[derive(Clone, Debug)]
pub struct PendingJob {
    pub job: Job,
    pub attempts: u32,
}

/// An in-memory FIFO queue of upload jobs with dedup by [`DedupKey`], stale-job pruning
/// when full, and rejection (with [`QueueFullErr`]) when full of fresh jobs.
pub struct Queue {
    jobs: VecDeque<PendingJob>,
    capacity: usize,
}

impl Queue {
    pub fn new(capacity: usize) -> Self {
        Self {
            jobs: VecDeque::new(),
            capacity,
        }
    }

    pub fn len(&self) -> usize {
        self.jobs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    pub fn contains(&self, key: &DedupKey) -> bool {
        self.jobs
            .iter()
            .any(|pending| pending.job.dedup_key() == *key)
    }

    /// Push a new job at the tail. Returns `Ok(Duplicate)` without modifying
    /// the queue when a key-equal job is already pending (dedup wins even when
    /// the queue is full). When full, stale jobs are pruned first; if the
    /// queue is still full, the enqueue is rejected with
    /// `UploadErr::QueueFullErr`.
    pub async fn enqueue(&mut self, job: Job) -> Result<Outcome, UploadErr> {
        if self.contains(&job.dedup_key()) {
            info!(
                "upload job for file {} (rule {}) is already queued; skipping enqueue",
                job.file, job.upload_rule_id
            );
            return Ok(Outcome::Duplicate);
        }
        self.reserve_slot(&job).await?;
        self.jobs.push_back(PendingJob { job, attempts: 0 });
        Ok(Outcome::Enqueued)
    }

    /// Push a previously popped job back at the tail, preserving its attempt
    /// count. No dedup check: the job's key was claimed as in-flight the
    /// entire time, so it cannot already be in the queue.
    pub async fn requeue(&mut self, pending: PendingJob) -> Result<(), UploadErr> {
        self.reserve_slot(&pending.job).await?;
        self.jobs.push_back(pending);
        Ok(())
    }

    pub fn pop_front(&mut self) -> Option<PendingJob> {
        self.jobs.pop_front()
    }

    /// Ensure at least one slot is free, pruning stale jobs if the queue is
    /// full. Errors with `QueueFullErr` when the queue remains full of fresh
    /// jobs.
    async fn reserve_slot(&mut self, job: &Job) -> Result<(), UploadErr> {
        if self.jobs.len() < self.capacity {
            return Ok(());
        }
        self.prune_stale().await;
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

    /// Drop every queued job whose source file no longer matches what the job
    /// recorded — the bytes the job described are gone, so it can never
    /// upload them. (`VecDeque::retain` cannot await, so the deque is
    /// rebuilt.)
    async fn prune_stale(&mut self) {
        let mut fresh = VecDeque::with_capacity(self.jobs.len());
        for pending in std::mem::take(&mut self.jobs) {
            match stale_reason(&pending.job).await {
                None => fresh.push_back(pending),
                Some(reason) => {
                    let job = &pending.job;
                    warn!(
                        "pruning stale upload job (rule {}, file {}, digest {}): {reason}",
                        job.upload_rule_id, job.file, job.digest
                    );
                }
            }
        }
        self.jobs = fresh;
    }
}

/// Returns why `job` is stale, or `None` when it is fresh. A transient IO
/// error (anything but NotFound) is not proof of staleness, so the job is
/// treated as fresh — dropping work on a hiccup is worse than a full queue.
async fn stale_reason(job: &Job) -> Option<String> {
    let md = match files::metadata(&job.file).await {
        Ok(md) => md,
        Err(FileSysErr::PathDoesNotExistErr(_)) => {
            return Some("file no longer exists".to_string());
        }
        Err(_) => return None,
    };
    if md.len() != job.size {
        return Some(format!("size changed from {} to {}", job.size, md.len()));
    }
    let mtime = DateTime::<Utc>::from(md.modified().unwrap_or(SystemTime::now()));
    if mtime != job.mtime {
        return Some(format!("mtime changed from {} to {mtime}", job.mtime));
    }
    None
}
