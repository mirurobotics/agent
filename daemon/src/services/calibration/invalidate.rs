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
    use crate::calibrations::{latest, LatestStore, Report};
    use crate::test_utils::calibrations::{latest_store, report};
    use crate::test_utils::filesys::dirs as test_dirs;

    // external crates
    use uuid::Uuid;

    /// A latest store under its own temp dir holding the `arm` report.
    async fn store_with_arm() -> (test_dirs::TempDir, LatestStore, Report) {
        let dir = test_dirs::temp("resolve_latest_entry").unwrap();
        let store = latest_store(&dir, latest::DEFAULT_CAPACITY).await;
        let arm = report("arm");
        store.put(arm.metadata.clone(), &arm.data).await.unwrap();
        (dir, store, arm)
    }

    #[test]
    fn a_blank_reason_is_refused() {
        for reason in ["", " \n"] {
            assert!(verify_reason(reason).is_err(), "{reason:?}");
        }
    }

    #[test]
    fn a_reason_over_512_chars_is_refused() {
        assert!(verify_reason(&"é".repeat(513)).is_err());
    }

    #[test]
    fn a_reason_up_to_512_chars_is_accepted() {
        assert!(verify_reason("power cycle").is_ok());
        assert!(verify_reason(&"é".repeat(512)).is_ok());
    }

    #[tokio::test]
    async fn the_component_resolves_to_its_entry() {
        let (_dir, store, arm) = store_with_arm().await;

        let entry = resolve_latest_entry(&store, "arm".to_string(), None)
            .await
            .unwrap();

        assert_eq!(entry, arm.metadata);
    }

    #[tokio::test]
    async fn the_entry_resolves_when_its_report_id_is_given() {
        let (_dir, store, arm) = store_with_arm().await;
        let report_id = Some(arm.metadata.report_id);

        let entry = resolve_latest_entry(&store, "arm".to_string(), report_id)
            .await
            .unwrap();

        assert_eq!(entry, arm.metadata);
    }

    #[tokio::test]
    async fn a_component_without_an_entry_is_not_found() {
        let (_dir, store, _) = store_with_arm().await;

        let err = resolve_latest_entry(&store, "leg".to_string(), None)
            .await
            .unwrap_err();

        assert_eq!(err.to_string(), "no latest calibration of leg matches");
    }

    #[tokio::test]
    async fn a_report_id_that_isnt_the_entry_is_not_found() {
        let (_dir, store, _) = store_with_arm().await;
        let report_id = Some(Uuid::new_v4());

        let err = resolve_latest_entry(&store, "arm".to_string(), report_id)
            .await
            .unwrap_err();

        assert_eq!(err.to_string(), "no latest calibration of arm matches");
    }
}
