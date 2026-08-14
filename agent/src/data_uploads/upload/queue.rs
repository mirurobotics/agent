// internal crates
use crate::data_uploads::{queue, upload::job::Job};

// external crates
use chrono::{DateTime, Utc};

impl queue::QueueJob for Job {
    const LABEL: &'static str = "upload";

    /// An upload job is actionable the moment it is enqueued.
    fn due_at(&self) -> DateTime<Utc> {
        DateTime::<Utc>::MIN_UTC
    }

    fn name(&self) -> String {
        self.file.to_string()
    }
}

/// The uploader's job queue: the shared [`queue::Queue`] carrying upload jobs.
pub type Queue = queue::Queue<Job>;

pub type QueueEntry = queue::QueueEntry<Job>;

pub type QueueSnapshot = queue::QueueSnapshot<Job>;

pub type QueueSnapshotFile = queue::QueueSnapshotFile<Job>;
