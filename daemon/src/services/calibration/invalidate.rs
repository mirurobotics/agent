// internal crates
use crate::calibrations::{ReportMetadata, Uploader};
use crate::services::errors::ServiceErr;

// external crates
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// What a device app asks to invalidate, and why.
#[derive(Clone, Debug, PartialEq)]
pub struct InvalidateParams {
    /// The flat component, which the latest calibrations are keyed by.
    pub component: String,
    /// When set, the component's latest calibration must be this report.
    pub report_id: Option<Uuid>,
    pub reason: String,
}

/// Mark the component's latest calibration invalid at `now` and queue the
/// invalidation for the control plane, unless it already is invalid, and
/// return it. Not found when the component has no latest calibration, or
/// `report_id` is set and isn't it. Nothing here waits on the control plane.
pub async fn invalidate(
    cal_uploader: &Uploader,
    params: InvalidateParams,
    now: DateTime<Utc>,
) -> Result<ReportMetadata, ServiceErr> {
    let invalidated = cal_uploader
        .invalidate(params.component, params.report_id, params.reason, now)
        .await?;
    Ok(invalidated)
}
