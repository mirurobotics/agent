// internal crates
use crate::data_uploads::retention::job::Job;
use crate::filesys::state_file::SingleThreadStateFile;
use crate::models::Patch;

// external crates
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DeleteQueueSnapshot {
    pub entries: Vec<Job>,
}

impl Patch<DeleteQueueSnapshot> for DeleteQueueSnapshot {
    fn patch(&mut self, patch: DeleteQueueSnapshot) {
        *self = patch;
    }
}

/// The queue's persistence handle: an atomically-written JSON snapshot with an
/// in-memory cache. Mirrors the uploader's `QueueSnapshotFile`.
pub type DeleteQueueSnapshotFile = SingleThreadStateFile<DeleteQueueSnapshot, DeleteQueueSnapshot>;
