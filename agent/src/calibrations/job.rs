// internal crates
use crate::buffer::BufferJob;
use crate::queue;

// external crates
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One buffered calibration report, as the upload worker will send it to Miru.
/// Everything here is captured at intake, so an upload hours later still
/// reports the deployment and time the calibration was taken under. The
/// content itself lives in the report file named by the queue entry's id
/// (see [`crate::buffer::Buffer`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalibrationJob {
    pub component_slug: String,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub aspect: Option<String>,
    #[serde(default)]
    pub serial_number: Option<String>,
    /// The deployment active at intake, or `None` when there was none.
    #[serde(default)]
    pub deployment_id: Option<String>,
    /// When the agent received the calibration.
    pub calibrated_at: DateTime<Utc>,
}

impl queue::QueueJob for CalibrationJob {
    const LABEL: &'static str = "calibration";

    /// A calibration is uploadable the moment it is buffered.
    fn due_at(&self) -> DateTime<Utc> {
        DateTime::<Utc>::MIN_UTC
    }

    fn name(&self) -> String {
        format!(
            "calibration of {} received at {}",
            self.component_slug,
            self.calibrated_at.to_rfc3339()
        )
    }
}

impl BufferJob for CalibrationJob {
    fn received_at(&self) -> DateTime<Utc> {
        self.calibrated_at
    }
}

pub type Queue = queue::Queue<CalibrationJob>;

pub type QueueEntry = queue::QueueEntry<CalibrationJob>;

pub type QueueSnapshot = queue::QueueSnapshot<CalibrationJob>;

pub type QueueSnapshotFile = queue::QueueSnapshotFile<CalibrationJob>;
