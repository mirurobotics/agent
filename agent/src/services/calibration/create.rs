// internal crates
use crate::calibrations::{Report, ReportMetadata, Uploader};
use crate::disk;
use crate::models::InstanceFormat;
use crate::services::errors::ServiceErr;

// external crates
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// A calibration as an on-device application reports it.
#[derive(Clone, Debug, PartialEq)]
pub struct CreateParams {
    pub component: String,
    pub role: Option<String>,
    pub aspect: Option<String>,
    pub serial_number: Option<String>,
    pub reported_by: Option<String>,
    /// The format `data` is serialized in.
    pub format: InstanceFormat,
    /// The calibration's values, queued exactly as given.
    pub data: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Created {
    /// The report's id, sent to Miru with every upload of it.
    pub report_id: Uuid,
    /// When the agent received the report; Miru records it as `reported_at`.
    pub reported_at: DateTime<Utc>,
}

/// Accept a calibration: stamp it with `now` and the deployed deployment, if
/// any, and hand it to the uploader to queue. Nothing here waits on Miru. A
/// deployments-cache error fails the call rather than dropping the
/// deployment.
pub async fn create(
    uploader: &Uploader,
    deployments: &disk::Deployments,
    params: CreateParams,
    now: DateTime<Utc>,
) -> Result<Created, ServiceErr> {
    let deployment_id = disk::deployments::find_deployed(deployments)
        .await?
        .map(|deployment| deployment.id);
    let report_id = Uuid::new_v4();
    let report = Report {
        metadata: ReportMetadata {
            report_id,
            component: params.component,
            format: params.format,
            role: params.role,
            aspect: params.aspect,
            serial_number: params.serial_number,
            reported_by: params.reported_by,
            deployment_id,
            reported_at: now,
        },
        data: params.data,
    };
    uploader.enqueue(report).await?;
    Ok(Created {
        report_id,
        reported_at: now,
    })
}
