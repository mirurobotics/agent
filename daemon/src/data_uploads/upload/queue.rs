// internal crates
use crate::data_uploads::upload::job::Job;
use crate::queue;

// external crates
use chrono::{DateTime, Utc};

impl queue::QueueJob for Job {
    const LABEL: &'static str = "upload";

    /// An upload job is actionable the moment it is enqueued.
    fn ready_at(&self) -> DateTime<Utc> {
        DateTime::<Utc>::MIN_UTC
    }

    fn name(&self) -> String {
        self.file.to_string()
    }
}

/// The uploader's job queue: the shared [`queue::FileQueue`] carrying upload jobs.
pub type Queue = queue::FileQueue<Job>;

pub type QueueEntry = queue::QueueEntry<Job>;

pub type QueueSnapshot = queue::QueueSnapshot<Job>;

pub type QueueSnapshotFile = queue::QueueSnapshotFile<Job>;
