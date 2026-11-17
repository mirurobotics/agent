// internal crates
use crate::calibrations::errors::InvalidReasonErr;
use crate::trace;

// external crates
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::warn;
use uuid::Uuid;

/// Which report a device app marked invalid, when and why.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Invalidation {
    pub report_id: Uuid,
    pub invalidated_at: DateTime<Utc>,
    pub reason: String,
}

/// The longest reason the control plane accepts, in characters.
const MAX_REASON_CHARS: usize = 512;

/// Reject a reason the control plane would refuse, so the device never keeps
/// an invalidation that can't be uploaded.
pub(crate) fn verify_reason(reason: &str) -> Result<(), InvalidReasonErr> {
    if !reason.trim().is_empty() && reason.chars().count() <= MAX_REASON_CHARS {
        return Ok(());
    }
    let err = InvalidReasonErr {
        max_chars: MAX_REASON_CHARS,
        trace: trace!(),
    };
    warn!("calibrations: rejecting invalidation: {err}");
    Err(err)
}
