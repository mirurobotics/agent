// internal crates
use crate::filesys::state_file_impl::StateFileImpl;
use crate::models::Patch;

// external crates
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueueEntry<J> {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    pub job: J,
    #[serde(default)]
    pub attempts: u32,
    /// Earliest instant this entry is eligible; `None` means "eligible now".
    #[serde(default)]
    pub next_attempt_at: Option<DateTime<Utc>>,
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
pub type QueueSnapshotFile<J> = StateFileImpl<QueueSnapshot<J>, QueueSnapshot<J>>;
