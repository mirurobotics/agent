// internal crates
use crate::calibrations::{Report, ReportMetadata, Uploader};
use crate::disk;
use crate::services::errors::ServiceErr;

// external crates
use chrono::{DateTime, Utc};

/// A calibration as an on-device application reports it.
#[derive(Clone, Debug, PartialEq)]
pub struct CreateParams {
    pub component_slug: String,
    pub role: Option<String>,
    pub aspect: Option<String>,
    pub serial_number: Option<String>,
    /// The `{ format, data }` instance content, buffered exactly as given.
    pub content: serde_json::Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Created {
    /// When the agent received the report; Miru records it as `reported_at`.
    pub reported_at: DateTime<Utc>,
}

/// Accept a calibration: stamp it with `now` and the deployed deployment (if
/// any) and hand it to the uploader, which buffers it. Works offline; nothing
/// here waits on Miru. A deployments-cache error fails the call rather than
/// dropping the deployment association.
pub async fn create(
    uploader: &Uploader,
    deployments: &disk::Deployments,
    params: CreateParams,
    now: DateTime<Utc>,
) -> Result<Created, ServiceErr> {
    let deployment_id = disk::deployments::find_deployed(deployments)
        .await?
        .map(|deployment| deployment.id);
    let report = Report {
        metadata: ReportMetadata {
            component_slug: params.component_slug,
            role: params.role,
            aspect: params.aspect,
            serial_number: params.serial_number,
            deployment_id,
            calibrated_at: now,
        },
        content: params.content,
    };
    uploader.enqueue(report).await?;
    Ok(Created { reported_at: now })
}
