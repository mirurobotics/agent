//! Listing the latest calibrations that intake wrote through.

// internal crates
use crate::mocks::calibration_sender::Step;
use crate::services::calibration::helpers::{params, Fixture};
use crate::test_utils::calibrations::received_at;
use miru_agent::calibrations::{LatestCalibrations, LatestFilter, ListedEntry, ReportMetadata};
use miru_agent::models::{CalibrationUploadStatus, InstanceFormat};
use miru_agent::services::calibration::{self, CreateParams};
use miru_agent::services::ServiceErr;

// external crates
use tokio::sync::oneshot;
use uuid::Uuid;

#[tokio::test]
async fn list_latest_includes_a_created_report() {
    let f = Fixture::new().await;
    // the report stays queued while its upload hangs
    let (_hold, hang) = oneshot::channel();
    f.sender.push(Step::Hang(hang));
    let created = f.create(params()).await.unwrap();

    let latest = calibration::list_latest(&f.uploader, LatestFilter::default()).await;

    let metadata = ReportMetadata {
        report_id: created.report_id,
        component: "arm".to_string(),
        format: InstanceFormat::Yaml,
        role: Some("left".to_string()),
        aspect: Some("pose".to_string()),
        serial_number: Some("SN-1".to_string()),
        reported_by: Some("Ada Lovelace".to_string()),
        deployment_id: None,
        reported_at: received_at(),
    };
    let entry = ListedEntry {
        metadata,
        upload_status: CalibrationUploadStatus::Queued,
    };
    let want = LatestCalibrations {
        is_confirmed: false,
        entries: vec![entry],
    };
    assert_eq!(latest.unwrap(), want);
}

#[tokio::test]
async fn list_latest_applies_the_filter() {
    let f = Fixture::new().await;
    f.create(params()).await.unwrap();
    let leg = CreateParams {
        component: "leg".to_string(),
        ..params()
    };
    let created = f.create(leg).await.unwrap();
    let filter = LatestFilter {
        component: vec!["leg".to_string()],
        ..LatestFilter::default()
    };

    let latest = calibration::list_latest(&f.uploader, filter).await;

    let listed = latest.unwrap().entries.into_iter();
    let ids: Vec<Uuid> = listed.map(|entry| entry.metadata.report_id).collect();
    assert_eq!(ids, [created.report_id]);
}

#[tokio::test]
async fn list_latest_fails_once_the_uploader_stops() {
    let f = Fixture::new().await;
    f.uploader.shutdown().await.unwrap();

    let result = calibration::list_latest(&f.uploader, LatestFilter::default()).await;

    let err = result.unwrap_err();
    assert!(matches!(err, ServiceErr::CalibrationsErr(_)), "{err:?}");
}
