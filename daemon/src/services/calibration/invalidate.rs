// internal crates
use crate::calibrations::{Invalidation, LatestStore, ReportMetadata};
use crate::services::{calibration::content::parse_report_id, errors::ServiceErr};

// external crates
use chrono::{DateTime, Utc};

/// Mark the latest calibration with `report_id` invalid at `now` for `reason`,
/// unless it already is, and return it. A report a newer one replaced is not
/// found. Only the device's store records it; nothing is sent to the control
/// plane yet.
pub async fn invalidate(
    store: &LatestStore,
    report_id: &str,
    reason: String,
    now: DateTime<Utc>,
) -> Result<ReportMetadata, ServiceErr> {
    let report_id = parse_report_id(report_id)?;
    let invalidation = Invalidation {
        invalidated_at: now,
        reason,
    };
    Ok(store.invalidate(report_id, invalidation).await?)
}
