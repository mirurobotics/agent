// standard crates
use std::collections::VecDeque;

// internal crates
use crate::trace;
use crate::upload::{
    errors::{QueueFullErr, UploadErr},
    job::Job,
};

// external crates
use tracing::warn;

#[derive(Clone, Debug)]
pub struct QueueEntry {
    pub job: Job,
    pub attempts: u32,
}

/// An in-memory FIFO queue of upload jobs with stale-job pruning when full, and
/// rejection (with [`QueueFullErr`]) when full of fresh jobs.
pub struct Queue {
    jobs: VecDeque<QueueEntry>,
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

    /// Push a new job at the tail. When full, stale jobs are pruned first; if
    /// the queue is still full, the enqueue is rejected with
    /// `UploadErr::QueueFullErr`.
    pub async fn enqueue(&mut self, job: Job) -> Result<(), UploadErr> {
        self.verify_capacity(&job).await?;
        self.jobs.push_back(QueueEntry { job, attempts: 0 });
        Ok(())
    }

    /// Push a previously popped job back at the tail, preserving its attempt
    /// count.
    pub async fn requeue(&mut self, entry: QueueEntry) -> Result<(), UploadErr> {
        self.verify_capacity(&entry.job).await?;
        self.jobs.push_back(entry);
        Ok(())
    }

    pub fn pop_front(&mut self) -> Option<QueueEntry> {
        self.jobs.pop_front()
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
}
