// external crates
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// When and why a device app marked a calibration invalid.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Invalidation {
    pub invalidated_at: DateTime<Utc>,
    pub reason: String,
}
