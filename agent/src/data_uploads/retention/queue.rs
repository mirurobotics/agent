// internal crates
use crate::data_uploads::{queue, retention::job::Job};

// external crates
use chrono::{DateTime, Utc};

impl queue::QueueJob for Job {
    const LABEL: &'static str = "delete";

    fn due_at(&self) -> DateTime<Utc> {
        Job::due_at(self)
    }

    fn name(&self) -> String {
        self.file.to_string()
    }
}

/// The retention deleter's job queue: the shared [`queue::Queue`] carrying
/// delete jobs.
pub type Queue = queue::Queue<Job>;

pub type QueueEntry = queue::QueueEntry<Job>;

pub type DeleteQueueSnapshot = queue::QueueSnapshot<Job>;

pub type DeleteQueueSnapshotFile = queue::QueueSnapshotFile<Job>;
