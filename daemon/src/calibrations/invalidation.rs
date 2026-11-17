// external crates
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Which report a device app marked invalid, when and why.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Invalidation {
    pub report_id: Uuid,
    pub invalidated_at: DateTime<Utc>,
    pub reason: String,
}
