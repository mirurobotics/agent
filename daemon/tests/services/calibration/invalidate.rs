//! Marking a component's latest calibration invalid and queueing the
//! invalidation's upload.

// internal crates
use crate::services::calibration::helpers::{params, Fixture};
use crate::test_utils::calibrations::received_at;
use miru_daemon::calibrations::{CalibrationsErr, Invalidation};
use miru_daemon::services::calibration::{self, InvalidateParams};
use miru_daemon::services::ServiceErr;

// external crates
use chrono::TimeDelta;
use uuid::Uuid;

/// The flat component `params()` is reported under.
const COMPONENT: &str = "arm-left-pose";

fn invalidate_params(component: &str, report_id: Option<Uuid>) -> InvalidateParams {
    InvalidateParams {
        component: component.to_string(),
        report_id,
        reason: "power cycle".to_string(),
    }
}

#[tokio::test]
async fn invalidate_stamps_the_latest_and_uploads_the_invalidation() {
    for with_report_id in [false, true] {
        let mut f = Fixture::new().await;
        let created = f.create(params()).await.unwrap();
        f.uploaded().await;
        let report_id = with_report_id.then_some(created.report_id);
        let now = received_at() + TimeDelta::hours(1);

        let invalidated = calibration::invalidate(
            &f.cal_uploader,
            invalidate_params(COMPONENT, report_id),
            now,
        )
        .await
        .unwrap();

        assert_eq!(invalidated.report_id, created.report_id);
        assert_eq!(invalidated.invalidated_at, Some(now));
        let expected = Invalidation {
            report_id: created.report_id,
            invalidated_at: now,
            reason: "power cycle".to_string(),
        };
        let uploaded = f.uploaded_invalidation().await;
        assert_eq!(uploaded, expected);
    }
}

#[tokio::test]
async fn invalidate_of_an_unknown_component_or_a_stale_report_is_not_found() {
    let f = Fixture::new().await;
    f.create(params()).await.unwrap();

    let cases = [("leg", None), (COMPONENT, Some(Uuid::new_v4()))];
    for (component, report_id) in cases {
        let result = calibration::invalidate(
            &f.cal_uploader,
            invalidate_params(component, report_id),
            received_at(),
        )
        .await;

        let err = result.unwrap_err();
        assert!(
            matches!(
                err,
                ServiceErr::CalibrationsErr(CalibrationsErr::LatestNotFoundErr(_))
            ),
            "{component}: {err:?}"
        );
    }
}
