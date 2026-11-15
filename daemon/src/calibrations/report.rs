// internal crates
use crate::calibrations::errors::{CalibrationsErr, InvalidReportErr};
use crate::filesys::{files, Dir, File, FileSysErr};
use crate::models::InstanceFormat;
use crate::queue;
use crate::trace;
use backend_api::models as backend_client;

// external crates
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::error;
use uuid::Uuid;

/// A calibration report: what intake hands the uploader and what the uploader
/// sends to the control plane. The metadata is queued in `queue.json`; the data
/// is kept in its own file, `reports/<entry id>`.
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
    /// Minted at intake and sent with every upload, so the control plane
    /// records the report once.
    #[serde(default = "Uuid::new_v4")]
    pub report_id: Uuid,
    pub component: String,
    /// The format [`Report::data`] is serialized in.
    pub format: InstanceFormat,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub aspect: Option<String>,
    #[serde(default)]
    pub serial_number: Option<String>,
    #[serde(default)]
    pub reported_by: Option<String>,
    /// The deployment active at intake, or `None` when there was none.
    #[serde(default)]
    pub deployment_id: Option<String>,
    /// When the agent received the calibration.
    pub reported_at: DateTime<Utc>,
}

/// A report's component, role and aspect: the store keeps one entry per key.
pub(crate) type Key<'a> = (&'a str, Option<&'a str>, Option<&'a str>);

impl ReportMetadata {
    pub(crate) fn key(&self) -> Key<'_> {
        (
            self.component.as_str(),
            self.role.as_deref(),
            self.aspect.as_deref(),
        )
    }

    /// Whether `self` is for `other`'s key and was reported at the same time
    /// or later, so a merge keeps it over `other`.
    pub(crate) fn is_as_new_as(&self, other: &ReportMetadata) -> bool {
        self.key() == other.key() && self.reported_at >= other.reported_at
    }
}

impl From<backend_client::Calibration> for ReportMetadata {
    fn from(calibration: backend_client::Calibration) -> Self {
        Self {
            report_id: calibration.report_id,
            component: calibration.component_slug,
            format: (&calibration.format).into(),
            role: calibration.role_slug,
            aspect: calibration.aspect_slug,
            serial_number: calibration.serial_number,
            reported_by: calibration.reported_by.flatten().map(|u| u.identifier),
            deployment_id: calibration.deployment_id,
            reported_at: calibration
                .reported_at
                .parse::<DateTime<Utc>>()
                .unwrap_or_else(|e| {
                    error!("Error parsing reported_at: {e}");
                    DateTime::<Utc>::UNIX_EPOCH
                }),
        }
    }
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

/// The file holding the data of the report queued as `entry_id`.
pub(crate) fn report_file(reports: &Dir, entry_id: Uuid) -> File {
    reports.file(&report_file_name(entry_id))
}

/// The name of the file holding the data of the report queued as `entry_id`.
pub(crate) fn report_file_name(entry_id: Uuid) -> String {
    entry_id.hyphenated().to_string()
}

/// The data in report `file`. A missing file or one that isn't UTF-8 can
/// never be sent, so either is a terminal [`InvalidReportErr`]; any other
/// read error is returned as is.
pub(crate) async fn read_data(file: &File) -> Result<String, CalibrationsErr> {
    match files::read_string(file).await {
        Ok(data) => Ok(data),
        Err(FileSysErr::PathDoesNotExistErr(_)) => Err(invalid("the report file is missing")),
        Err(FileSysErr::ConvertUTF8Err(e)) => {
            Err(invalid(&format!("the report file is not UTF-8: {e}")))
        }
        Err(e) => Err(e.into()),
    }
}

fn invalid(reason: &str) -> CalibrationsErr {
    CalibrationsErr::InvalidReportErr(InvalidReportErr {
        reason: reason.to_string(),
        trace: trace!(),
    })
}

pub type Queue = queue::FileQueue<ReportMetadata>;

pub type QueueEntry = queue::QueueEntry<ReportMetadata>;

pub type QueueSnapshot = queue::QueueSnapshot<ReportMetadata>;

pub type QueueSnapshotFile = queue::QueueSnapshotFile<ReportMetadata>;

#[cfg(test)]
mod tests {
    // internal crates
    use crate::test_utils::calibrations::reported;
    use miru_daemon::calibrations::ReportMetadata;

    // external crates
    use chrono::TimeDelta;
    use uuid::Uuid;

    /// `metadata` reported `secs` later, as another report.
    fn reported_after(metadata: &ReportMetadata, secs: i64) -> ReportMetadata {
        ReportMetadata {
            report_id: Uuid::new_v4(),
            reported_at: metadata.reported_at + TimeDelta::seconds(secs),
            ..metadata.clone()
        }
    }

    // ============================ IS AS NEW AS ============================ //
    #[test]
    fn is_as_new_as_a_report_for_its_key_at_the_same_time_or_earlier() {
        let arm = reported("arm", None, None);

        for secs in [0, 1] {
            assert!(reported_after(&arm, secs).is_as_new_as(&arm), "{secs}");
        }
    }

    #[test]
    fn is_not_as_new_as_a_later_report_for_its_key() {
        let arm = reported("arm", None, None);

        assert!(!reported_after(&arm, -1).is_as_new_as(&arm));
    }

    #[test]
    fn is_not_as_new_as_a_report_for_another_key() {
        let arm = reported("arm", None, None);
        let others = [
            reported("leg", None, None),
            reported("arm", Some("left"), None),
            reported("arm", None, Some("pose")),
        ];

        for other in &others {
            let later = ReportMetadata {
                reported_at: arm.reported_at + TimeDelta::seconds(1),
                ..other.clone()
            };
            assert!(!later.is_as_new_as(&arm), "{other:?}");
        }
    }
}
