// internal crates
use crate::data_uploads::{
    queue,
    upload::{
        errors::{QueueFullErr, UploadErr},
        job::Job,
    },
};
use crate::trace;

// external crates
use chrono::{DateTime, Utc};

impl queue::QueueJob for Job {
    type QueueFullErr = UploadErr;

    const LABEL: &'static str = "upload";

    /// An upload job is actionable the moment it is enqueued.
    fn due_at(&self) -> DateTime<Utc> {
        DateTime::<Utc>::MIN_UTC
    }

    fn file(&self) -> String {
        self.file.to_string()
    }

    fn queue_full_err(capacity: usize, file: String) -> UploadErr {
        UploadErr::QueueFullErr(QueueFullErr {
            capacity,
            file,
            trace: trace!(),
        })
    }
}

/// The uploader's job queue: the shared [`queue::Queue`] carrying upload jobs.
pub type Queue = queue::Queue<Job>;

pub type QueueEntry = queue::QueueEntry<Job>;

pub type QueueSnapshot = queue::QueueSnapshot<Job>;

/// The queue's persistence handle: an atomically-written JSON snapshot with an
/// in-memory cache. Mirrors the scanner's `ScanSnapshotFile`.
pub type QueueSnapshotFile = queue::QueueSnapshotFile<Job>;
