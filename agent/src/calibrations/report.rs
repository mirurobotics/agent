// internal crates
use crate::models::InstanceFormat;
use crate::queue;

// external crates
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A calibration report: what intake hands the uploader and what the uploader
/// sends to Miru. The metadata is queued in `queue.json`; the data is kept in
/// its own file, `reports/<entry id>`.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub metadata: ReportMetadata,
    /// The calibration's values, serialized in [`ReportMetadata::format`] and
    /// kept exactly as given.
    pub data: String,
}

/// A queued report's metadata. Everything here is captured at intake, so an
/// upload hours later still reports the deployment and time the calibration
/// was taken under.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReportMetadata {
    pub component: String,
    /// The format [`Report::data`] is serialized in.
    pub format: InstanceFormat,
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
    pub reported_at: DateTime<Utc>,
}

impl queue::QueueJob for ReportMetadata {
    const LABEL: &'static str = "calibration";

    /// A calibration is uploadable the moment it is queued.
    fn ready_at(&self) -> DateTime<Utc> {
        DateTime::<Utc>::MIN_UTC
    }

    fn name(&self) -> String {
        let mut name = format!("calibration of {}", self.component);
        if let Some(role) = &self.role {
            name.push_str(&format!(" (role {role})"));
        }
        if let Some(aspect) = &self.aspect {
            name.push_str(&format!(" (aspect {aspect})"));
        }
        format!("{name} received at {}", self.reported_at.to_rfc3339())
    }
}

pub type Queue = queue::FileQueue<ReportMetadata>;

pub type QueueEntry = queue::QueueEntry<ReportMetadata>;

pub type QueueSnapshot = queue::QueueSnapshot<ReportMetadata>;

pub type QueueSnapshotFile = queue::QueueSnapshotFile<ReportMetadata>;
