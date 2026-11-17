// internal crates
use crate::calibrations::{Filter, Invalidation, LatestStore, ReportMetadata, Uploader};
use crate::services::errors::{InvalidReasonErr, LatestCalibrationNotFoundErr, ServiceErr};
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

/// Queue the invalidation of the component's latest calibration at `now` for
/// the control plane and mark it invalid, unless it already is, and return it.
/// Not found when the component has no latest calibration, or `report_id` is
/// set and isn't it. Nothing here waits on the control plane.
pub async fn invalidate(
    cal_uploader: &Uploader,
    latest_cals: &LatestStore,
    params: InvalidateParams,
    now: DateTime<Utc>,
) -> Result<ReportMetadata, ServiceErr> {
    verify_reason(&params.reason)?;
    let entry = resolve_latest_entry(latest_cals, params.component, params.report_id).await?;
    if entry.invalidated_at.is_some() {
        return Ok(entry);
    }
    let invalidation = Invalidation {
        report_id: entry.report_id,
        invalidated_at: now,
        reason: params.reason,
    };
    cal_uploader.invalidate(invalidation).await?;
    Ok(ReportMetadata {
        invalidated_at: Some(now),
        ..entry
    })
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
async fn resolve_latest_entry(
    latest_cals: &LatestStore,
    component: String,
    report_id: Option<Uuid>,
) -> Result<ReportMetadata, ServiceErr> {
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
    use super::{resolve_latest_entry, verify_reason};
    use crate::calibrations::latest;
    use crate::test_utils::calibrations::{latest_store, report};
    use crate::test_utils::filesys::dirs as test_dirs;

    // external crates
    use uuid::Uuid;

    #[test]
    fn a_reason_must_not_be_blank_or_over_512_chars() {
        for reason in [String::new(), " \n".to_string(), "é".repeat(513)] {
            assert!(verify_reason(&reason).is_err(), "{reason:?}");
        }
        for reason in ["power cycle".to_string(), "é".repeat(512)] {
            assert!(verify_reason(&reason).is_ok(), "{reason:?}");
        }
    }

    #[tokio::test]
    async fn the_latest_entry_resolves_when_its_component_and_any_report_id_match() {
        let dir = test_dirs::temp("resolve_latest_entry").unwrap();
        let store = latest_store(&dir, latest::DEFAULT_CAPACITY).await;
        let arm = report("arm");
        store.put(arm.metadata.clone(), &arm.data).await.unwrap();

        for report_id in [None, Some(arm.metadata.report_id)] {
            let entry = resolve_latest_entry(&store, "arm".to_string(), report_id)
                .await
                .unwrap();
            assert_eq!(entry, arm.metadata);
        }
        for (component, report_id) in [("leg", None), ("arm", Some(Uuid::new_v4()))] {
            let err = resolve_latest_entry(&store, component.to_string(), report_id)
                .await
                .unwrap_err();
            let want = format!("no latest calibration of {component} matches");
            assert_eq!(err.to_string(), want);
        }
    }
}
