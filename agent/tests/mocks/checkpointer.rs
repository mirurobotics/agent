// standard crates
use std::sync::{Arc, Mutex};

// internal crates
use miru_agent::data_uploads::upload::{Checkpoint, Checkpointer};

/// Records every saved checkpoint, in order. Clones share the record.
#[derive(Clone, Default)]
pub struct RecordingCheckpointer {
    saved: Arc<Mutex<Vec<Checkpoint>>>,
}

impl RecordingCheckpointer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn saved(&self) -> Vec<Checkpoint> {
        self.saved.lock().unwrap().clone()
    }
}

impl Checkpointer for RecordingCheckpointer {
    async fn save(&self, checkpoint: Checkpoint) {
        self.saved.lock().unwrap().push(checkpoint);
    }
}
