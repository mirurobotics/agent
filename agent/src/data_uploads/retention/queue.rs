// internal crates
use crate::data_uploads::{
    queue,
    retention::{errors::*, job::Job},
};
use crate::trace;

// external crates
use chrono::{DateTime, Utc};

impl queue::QueueJob for Job {
    type QueueFullErr = DeleteErr;

    const LABEL: &'static str = "delete";

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

/// The retention deleter's job queue: the shared [`queue::Queue`] carrying
/// delete jobs.
pub type Queue = queue::Queue<Job>;

pub type QueueEntry = queue::QueueEntry<Job>;

pub type DeleteQueueSnapshot = queue::QueueSnapshot<Job>;

pub type DeleteQueueSnapshotFile = queue::QueueSnapshotFile<Job>;
