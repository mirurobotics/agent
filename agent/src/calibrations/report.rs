// internal crates
use crate::queue;

// external crates
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One queued calibration report, as the upload worker will send it to Miru.
/// Everything here is captured at intake, so an upload hours later still
/// reports the deployment and time the calibration was taken under. The
/// content itself lives in the report file named by the queue entry's id
/// (see [`crate::queue::DirQueue`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReportMetadata {
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

impl queue::QueueJob for ReportMetadata {
    const LABEL: &'static str = "calibration";

    /// A calibration is uploadable the moment it is queued.
    fn due_at(&self) -> DateTime<Utc> {
        DateTime::<Utc>::MIN_UTC
    }

    fn name(&self) -> String {
        let mut name = format!("calibration of {}", self.component_slug);
        if let Some(role) = &self.role {
            name.push_str(&format!(" (role {role})"));
        }
        if let Some(aspect) = &self.aspect {
            name.push_str(&format!(" (aspect {aspect})"));
        }
        format!("{name} received at {}", self.calibrated_at.to_rfc3339())
    }
}

pub type Queue = queue::FileQueue<ReportMetadata>;

pub type QueueEntry = queue::QueueEntry<ReportMetadata>;

pub type QueueSnapshot = queue::QueueSnapshot<ReportMetadata>;

pub type QueueSnapshotFile = queue::QueueSnapshotFile<ReportMetadata>;
