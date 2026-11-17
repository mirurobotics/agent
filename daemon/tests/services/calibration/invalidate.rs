//! Marking a component's latest calibration invalid and queueing the
//! invalidation's upload.

// internal crates
use crate::mocks::calibration_sender::Step;
use crate::services::calibration::helpers::{params, Fixture};
use crate::test_utils::calibrations::received_at;
use miru_daemon::calibrations::{latest, CalibrationsErr, Filter, Invalidation, UploaderOptions};
use miru_daemon::services::calibration::InvalidateParams;
use miru_daemon::services::ServiceErr;

// external crates
use chrono::TimeDelta;
use tokio::sync::oneshot;
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
        f.uploaded_report().await;
        let report_id = with_report_id.then_some(created.report_id);
        let now = received_at() + TimeDelta::hours(1);

        let invalidated = f
            .invalidate(invalidate_params(COMPONENT, report_id), now)
            .await
            .unwrap();

        assert_eq!(invalidated.report_id, created.report_id);
        assert_eq!(invalidated.invalidated_at, Some(now));
        let entries = f.latest.list(&Filter::default()).await.unwrap();
        assert_eq!(entries, [invalidated]);
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
async fn invalidate_of_an_invalid_calibration_returns_it_and_queues_nothing() {
    let mut f = Fixture::new().await;
    // the first invalidation's upload hangs, so it stays queued
    let (_hold, hang) = oneshot::channel();
    f.sender.push(Step::Ok);
    f.sender.push(Step::Hang(hang));
    f.create(params()).await.unwrap();
    let first = f
        .invalidate(invalidate_params(COMPONENT, None), received_at())
        .await
        .unwrap();
    f.uploaded_invalidation().await;
    let params = InvalidateParams {
        reason: "dropped".to_string(),
        ..invalidate_params(COMPONENT, None)
    };

    let again = f
        .invalidate(params, received_at() + TimeDelta::hours(1))
        .await
        .unwrap();

    assert_eq!(again, first);
    assert_eq!(f.cal_uploader.num_queued_uploads().await.unwrap(), 1);
}

#[tokio::test]
async fn invalidate_of_an_unknown_component_or_a_stale_report_is_not_found() {
    let f = Fixture::new().await;
    f.create(params()).await.unwrap();

    let cases = [("leg", None), (COMPONENT, Some(Uuid::new_v4()))];
    for (component, report_id) in cases {
        let err = f
            .invalidate(invalidate_params(component, report_id), received_at())
            .await
            .unwrap_err();

        assert!(
            matches!(err, ServiceErr::LatestCalibrationNotFoundErr(_)),
            "{component}: {err:?}"
        );
    }
}

#[tokio::test]
async fn invalidate_with_a_refused_reason_is_rejected_before_the_lookup() {
    let f = Fixture::new().await;
    let params = InvalidateParams {
        reason: " ".to_string(),
        ..invalidate_params("leg", None)
    };

    let err = f.invalidate(params, received_at()).await.unwrap_err();

    assert!(matches!(err, ServiceErr::InvalidReasonErr(_)), "{err:?}");
}

#[tokio::test]
async fn an_unreadable_latest_store_fails_invalidate() {
    let f = Fixture::new().await;
    f.corrupt_latest_index().await;

    let err = f
        .invalidate(invalidate_params(COMPONENT, None), received_at())
        .await
        .unwrap_err();

    assert!(
        matches!(
            err,
            ServiceErr::CalibrationsErr(CalibrationsErr::FileSysErr(_))
        ),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_full_upload_queue_fails_invalidate() {
    let options = UploaderOptions {
        capacity: 1,
        ..UploaderOptions::default()
    };
    let f = Fixture::with_options(options, latest::DEFAULT_CAPACITY).await;
    // the report's upload hangs, so it fills the queue
    let (_hold, hang) = oneshot::channel();
    f.sender.push(Step::Hang(hang));
    f.create(params()).await.unwrap();

    let err = f
        .invalidate(invalidate_params(COMPONENT, None), received_at())
        .await
        .unwrap_err();

    assert!(
        matches!(
            err,
            ServiceErr::CalibrationsErr(CalibrationsErr::QueueFullErr(_))
        ),
        "{err:?}"
    );
    let entries = f.latest.list(&Filter::default()).await.unwrap();
    assert_eq!(entries[0].invalidated_at, None);
    assert_eq!(f.cal_uploader.num_queued_uploads().await.unwrap(), 1);
}
