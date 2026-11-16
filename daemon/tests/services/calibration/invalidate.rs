//! Marking a latest calibration invalid by its `report_id`.

// internal crates
use crate::services::calibration::helpers::{params, Fixture};
use crate::test_utils::calibrations::received_at;
use miru_daemon::calibrations::{CalibrationsErr, Invalidation};
use miru_daemon::services::calibration;
use miru_daemon::services::ServiceErr;

// external crates
use chrono::TimeDelta;

#[tokio::test]
async fn invalidate_stamps_the_entry_with_the_time_and_reason() {
    let f = Fixture::new().await;
    let created = f.create(params()).await.unwrap();
    let report_id = created.report_id.to_string();
    let now = received_at() + TimeDelta::hours(1);

    let invalidated =
        calibration::invalidate(&f.latest, &report_id, "power cycle".to_string(), now).await;

    let expected = Invalidation {
        invalidated_at: now,
        reason: "power cycle".to_string(),
    };
    assert_eq!(invalidated.unwrap().invalidation, Some(expected));
}

#[tokio::test]
async fn invalidate_of_an_invalid_report_id_is_not_found() {
    let f = Fixture::new().await;

    let reason = "power cycle".to_string();

    let result = calibration::invalidate(&f.latest, "not-a-uuid", reason, received_at()).await;

    let err = result.unwrap_err();
    assert!(
        matches!(
            err,
            ServiceErr::CalibrationsErr(CalibrationsErr::NotFoundErr(_))
        ),
        "{err:?}"
    );
}
