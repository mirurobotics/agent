// internal crates
use crate::data_uploads::retention::job::Job;
use crate::queue;

// external crates
use chrono::{DateTime, Utc};

impl queue::QueueJob for Job {
    const LABEL: &'static str = "delete";

    /// The TTL deadline on the agent's uptime clock: the deleter selects
    /// with the uptime as `now`, never the system clock.
    fn ready_at(&self) -> DateTime<Utc> {
        Job::ready_at(self)
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
