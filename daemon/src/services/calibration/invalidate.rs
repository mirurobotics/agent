// internal crates
use crate::calibrations::{
    CalibrationsErr, Filter, Invalidation, LatestCalibrationNotFoundErr, LatestStore,
    ReportMetadata, Uploader,
};
use crate::services::errors::{InvalidReasonErr, ServiceErr};
use crate::trace;

// external crates
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// The longest reason the control plane accepts, in characters.
const MAX_REASON_CHARS: usize = 512;

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
    latest_cals: &LatestStore,
    params: InvalidateParams,
    now: DateTime<Utc>,
) -> Result<ReportMetadata, ServiceErr> {
    verify_reason(&params.reason)?;
    let entry = latest_entry(latest_cals, params.component, params.report_id).await?;
    if entry.invalidated_at.is_some() {
        return Ok(entry);
    }
    let invalidation = Invalidation {
        report_id: entry.report_id,
        invalidated_at: now,
        reason: params.reason,
    };
    Ok(cal_uploader.invalidate(invalidation).await?)
}

/// Reject a reason the control plane would refuse, so the device never keeps
/// an invalidation that can't be uploaded.
fn verify_reason(reason: &str) -> Result<(), InvalidReasonErr> {
    if !reason.trim().is_empty() && reason.chars().count() <= MAX_REASON_CHARS {
        return Ok(());
    }
    Err(InvalidReasonErr {
        max_chars: MAX_REASON_CHARS,
        trace: trace!(),
    })
}

/// `component`'s latest entry, if `report_id` is unset or names it.
async fn latest_entry(
    latest_cals: &LatestStore,
    component: String,
    report_id: Option<Uuid>,
) -> Result<ReportMetadata, CalibrationsErr> {
    let filter = Filter {
        component: vec![component.clone()],
        ..Filter::default()
    };
    // a key has at most one entry
    match latest_cals.list(&filter).await?.pop() {
        Some(entry) if report_id.is_none_or(|id| id == entry.report_id) => Ok(entry),
        _ => Err(LatestCalibrationNotFoundErr {
            component,
            trace: trace!(),
        }
        .into()),
    }
}

#[cfg(test)]
mod tests {
    // internal crates
    use super::verify_reason;

    #[test]
    fn a_reason_must_not_be_blank_or_over_512_chars() {
        for reason in [String::new(), " \n".to_string(), "é".repeat(513)] {
            assert!(verify_reason(&reason).is_err(), "{reason:?}");
        }
        for reason in ["power cycle".to_string(), "é".repeat(512)] {
            assert!(verify_reason(&reason).is_ok(), "{reason:?}");
        }
    }
}
