// internal crates
use crate::data_uploads::retention::job::Job;
use crate::queue;

// external crates
use chrono::{DateTime, Utc};

impl queue::QueueJob for Job {
    const LABEL: &'static str = "delete";

    /// Always ready as far as the queue can tell. Whether the TTL has run
    /// out depends on server and monotonic time, which the deleter tracks, so
    /// the deleter decides that itself (see `DeleterImpl::sweep`).
    fn ready_at(&self) -> DateTime<Utc> {
        DateTime::<Utc>::MIN_UTC
    }

    fn name(&self) -> String {
        self.file.to_string()
    }
}

/// The retention deleter's job queue: the shared [`queue::FileQueue`] carrying
/// delete jobs.
pub type Queue = queue::FileQueue<Job>;

pub type QueueEntry = queue::QueueEntry<Job>;

pub type DeleteQueueSnapshot = queue::QueueSnapshot<Job>;

pub type DeleteQueueSnapshotFile = queue::QueueSnapshotFile<Job>;
