// internal crates
use crate::calibrations::report::ReportMetadata;
use crate::queue;

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

impl queue::QueueJob for Job {
    const LABEL: &'static str = "calibration";

    /// A calibration is uploadable the moment it is queued.
    fn ready_at(&self) -> DateTime<Utc> {
        DateTime::<Utc>::MIN_UTC
    }

    fn name(&self) -> String {
        match self {
            Job::Report(report) => report_name(report),
        }
    }
}

fn report_name(report: &ReportMetadata) -> String {
    let mut name = format!("calibration of {}", report.component);
    if let Some(role) = &report.role {
        name.push_str(&format!(" (role {role})"));
    }
    if let Some(aspect) = &report.aspect {
        name.push_str(&format!(" (aspect {aspect})"));
    }
    format!("{name} received at {}", report.reported_at.to_rfc3339())
}

pub type Queue = queue::FileQueue<Job>;

pub type QueueEntry = queue::QueueEntry<Job>;

pub type QueueSnapshot = queue::QueueSnapshot<Job>;

pub type QueueSnapshotFile = queue::QueueSnapshotFile<Job>;
