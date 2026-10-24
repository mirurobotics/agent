// standard crates
use std::sync::{Arc, Mutex};

// internal crates
use miru_agent::data_uploads::upload::{Checkpointer, UploadRef};

/// Records every saved upload, in order. Clones share the record.
#[derive(Clone, Default)]
pub struct RecordingCheckpointer {
    saved: Arc<Mutex<Vec<UploadRef>>>,
}

impl RecordingCheckpointer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn saved(&self) -> Vec<UploadRef> {
        self.saved.lock().unwrap().clone()
    }
}

impl Checkpointer for RecordingCheckpointer {
    async fn save(&self, upload: &UploadRef) {
        self.saved.lock().unwrap().push(upload.clone());
    }
}

/// Discards every save, for tests that don't look at them.
pub struct NoopCheckpointer;

impl Checkpointer for NoopCheckpointer {
    async fn save(&self, _upload: &UploadRef) {}
}
