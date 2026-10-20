// internal crates
use crate::calibrations::{CalibrationJob, Report, SharedBuffer};
use crate::disk;
use crate::services::{calibration::validate, errors::ServiceErr};

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

/// Accept a calibration: validate it, stamp it with `now` and the deployed
/// deployment (if any), buffer it and wake the upload worker. Works offline;
/// nothing here talks to Miru. A deployments-cache error fails the call
/// rather than dropping the deployment association.
pub async fn create(
    buffer: &SharedBuffer,
    deployments: &disk::Deployments,
    params: CreateParams,
    now: DateTime<Utc>,
) -> Result<Created, ServiceErr> {
    validate::params(&params)?;
    let deployment_id = disk::deployments::find_deployed(deployments)
        .await?
        .map(|deployment| deployment.id);
    let report = Report {
        job: CalibrationJob {
            component_slug: params.component_slug,
            role: params.role,
            aspect: params.aspect,
            serial_number: params.serial_number,
            deployment_id,
            calibrated_at: now,
        },
        content: params.content,
    };
    buffer.enqueue(report).await?;
    Ok(Created { reported_at: now })
}
