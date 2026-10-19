// internal crates
use crate::queue;

// external crates
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One buffered calibration report, as the upload worker will send it to Miru.
/// Everything here is captured at intake, so an upload hours later still
/// reports the deployment and time the calibration was taken under. The
/// content itself lives in the report file named by `report_id`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalibrationJob {
    /// The agent-local receipt id, unique per device. Miru uses it to make
    /// the agent's own upload retries idempotent.
    pub report_id: String,
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
        self.report_id.clone()
    }
}

pub type Queue = queue::Queue<CalibrationJob>;

pub type QueueEntry = queue::QueueEntry<CalibrationJob>;

pub type QueueSnapshot = queue::QueueSnapshot<CalibrationJob>;

pub type QueueSnapshotFile = queue::QueueSnapshotFile<CalibrationJob>;
