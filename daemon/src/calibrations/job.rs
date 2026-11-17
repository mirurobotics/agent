// internal crates
use crate::calibrations::{invalidation::Invalidation, report::ReportMetadata};

// external crates
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A queued upload. A report is stored as its bare metadata, so a
/// `queue.json` from an older agent still loads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Job {
    /// Upload the report, whose data is in its report file.
    Report(ReportMetadata),
    /// Upload the invalidation of the report with `report_id`. It has no
    /// report file.
    Invalidation {
        report_id: Uuid,
        invalidation: Invalidation,
    },
}

impl Job {
    /// When the job was queued: when the agent received the report, or when
    /// the invalidation was made. Its age counts from here.
    pub(crate) fn queued_at(&self) -> DateTime<Utc> {
        match self {
            Job::Report(report) => report.reported_at,
            Job::Invalidation { invalidation, .. } => invalidation.invalidated_at,
        }
    }
}
