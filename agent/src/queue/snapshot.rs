// standard crates
use std::time::Instant;

// internal crates
use crate::filesys::state_file::StateFile;
use crate::models::Patch;

// external crates
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueueEntry<J> {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    pub job: J,
    #[serde(default)]
    pub attempts: u32,
    /// Monotonic instant before which this entry is not retried; `None` means
    /// "eligible now". Kept in memory only: snapshots never save it, and a
    /// `next_attempt_at` in a snapshot written by an older agent is ignored,
    /// so after a restart every entry is eligible right away.
    #[serde(skip)]
    pub next_attempt_at: Option<Instant>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueueSnapshot<J> {
    pub entries: Vec<QueueEntry<J>>,
}

// hand-written rather than derived: `#[derive(Default)]` would add a spurious
// `J: Default` bound that neither job type satisfies.
impl<J> Default for QueueSnapshot<J> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

impl<J> Patch<QueueSnapshot<J>> for QueueSnapshot<J> {
    fn patch(&mut self, patch: QueueSnapshot<J>) {
        *self = patch;
    }
}

/// The queue's persistence handle: an atomically-written JSON snapshot with an
/// in-memory cache. Mirrors the scanner's `ScanSnapshotFile`.
pub type QueueSnapshotFile<J> = StateFile<QueueSnapshot<J>, QueueSnapshot<J>>;
