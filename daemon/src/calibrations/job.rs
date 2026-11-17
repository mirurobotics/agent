// internal crates
use crate::calibrations::report::ReportMetadata;

// external crates
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A queued upload. A report is stored as its bare metadata, so a
/// `queue.json` from an older agent still loads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Job {
    /// Upload the report, whose data is in its report file.
    Report(ReportMetadata),
}

impl Job {
    /// When the job was queued. Its age counts from here.
    pub(crate) fn queued_at(&self) -> DateTime<Utc> {
        match self {
            Job::Report(report) => report.reported_at,
        }
    }
}
