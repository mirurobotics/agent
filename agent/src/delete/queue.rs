// internal crates
use crate::filesys::{state_file::SingleThreadStateFile, File};
use crate::models::Patch;

// external crates
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// A persisted pending-deletion record: "this exact file (path, size, mtime,
/// digest) became deletable at `eligible_at`; delete it once
/// `delete_delay_secs` have passed". Deliberately event-agnostic — it names
/// *when* the file became deletable, not *why* — so future retention policies
/// (openapi #212) add producers of this record, not fields.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PendingDelete {
    /// The exact file the executor uploaded. The worker only ever deletes
    /// paths carried by these records (path-safety invariant).
    pub file: File,
    /// Size/mtime/digest as recorded at upload time (from the `Job`).
    /// The sweep re-stats and deletes on a size+mtime match; when only the
    /// mtime differs it re-hashes and deletes only on a digest match (a
    /// touched-but-unchanged file is absorbed by the scanner as an mtime
    /// alias and never re-uploaded, so no replacement record would ever
    /// arrive).
    pub size: u64,
    pub mtime: DateTime<Utc>,
    pub digest: String,
    /// When the file became deletable (today: upload confirmation).
    /// Event-agnostic on purpose — future retention policies (#212)
    /// add producers, not fields.
    pub eligible_at: DateTime<Utc>,
    pub delete_delay_secs: i64,
    // for logging only
    pub upload_rule_id: String,
    pub deployment_id: String,
}

impl PendingDelete {
    /// The instant this record becomes actionable: `eligible_at` plus the
    /// delete delay (negative delays clamp to zero).
    pub fn due_at(&self) -> DateTime<Utc> {
        self.eligible_at + Duration::seconds(self.delete_delay_secs.max(0))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DeleteQueueSnapshot {
    pub entries: Vec<PendingDelete>,
}

impl Patch<DeleteQueueSnapshot> for DeleteQueueSnapshot {
    fn patch(&mut self, patch: DeleteQueueSnapshot) {
        *self = patch;
    }
}

/// The queue's persistence handle: an atomically-written JSON snapshot with an
/// in-memory cache. Mirrors the uploader's `QueueSnapshotFile`.
pub type DeleteQueueSnapshotFile = SingleThreadStateFile<DeleteQueueSnapshot, DeleteQueueSnapshot>;
