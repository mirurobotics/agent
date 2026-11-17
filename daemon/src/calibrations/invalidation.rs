// external crates
use chrono::{DateTime, Utc};

/// When and why a device app marked a calibration invalid.
#[derive(Clone, Debug, PartialEq)]
pub struct Invalidation {
    pub invalidated_at: DateTime<Utc>,
    pub reason: String,
}
