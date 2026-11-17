// internal crates
use crate::calibrations::{Invalidation, ReportMetadata, Uploader};
use crate::services::{calibration::parse_report_id, errors::ServiceErr};

// external crates
use chrono::{DateTime, Utc};

/// Mark the latest calibration with `report_id` invalid at `now` for `reason`
/// and queue the invalidation for the control plane, unless it already is
/// invalid, and return it. A report a newer one replaced is not found. Nothing
/// here waits on the control plane.
pub async fn invalidate(
    cal_uploader: &Uploader,
    report_id: &str,
    reason: String,
    now: DateTime<Utc>,
) -> Result<ReportMetadata, ServiceErr> {
    let report_id = parse_report_id(report_id)?;
    let invalidation = Invalidation {
        invalidated_at: now,
        reason,
    };
    Ok(cal_uploader.invalidate(report_id, invalidation).await?)
}
