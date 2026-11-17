//! Marking a latest calibration invalid by its `report_id` and queueing the
//! invalidation's upload.

// internal crates
use crate::services::calibration::helpers::{params, Fixture};
use crate::test_utils::calibrations::received_at;
use miru_daemon::calibrations::{CalibrationsErr, Invalidation};
use miru_daemon::services::calibration;
use miru_daemon::services::ServiceErr;

// external crates
use chrono::TimeDelta;
use uuid::Uuid;

#[tokio::test]
async fn invalidate_stamps_the_entry_and_uploads_the_invalidation() {
    let mut f = Fixture::new().await;
    let created = f.create(params()).await.unwrap();
    f.uploaded().await;
    let report_id = created.report_id.to_string();
    let now = received_at() + TimeDelta::hours(1);

    let invalidated =
        calibration::invalidate(&f.cal_uploader, &report_id, "power cycle".to_string(), now).await;

    let expected = Invalidation {
        invalidated_at: now,
        reason: "power cycle".to_string(),
    };
    assert_eq!(invalidated.unwrap().invalidation, Some(expected.clone()));
    let uploaded = f.uploaded_invalidation().await;
    assert_eq!(uploaded, (created.report_id, expected));
}

#[tokio::test]
async fn invalidate_of_an_invalid_or_unknown_report_id_is_not_found() {
    let f = Fixture::new().await;

    for report_id in ["not-a-uuid".to_string(), Uuid::new_v4().to_string()] {
        let reason = "power cycle".to_string();
        let result =
            calibration::invalidate(&f.cal_uploader, &report_id, reason, received_at()).await;

        let err = result.unwrap_err();
        assert!(
            matches!(
                err,
                ServiceErr::CalibrationsErr(CalibrationsErr::NotFoundErr(_))
            ),
            "{report_id}: {err:?}"
        );
    }
}
