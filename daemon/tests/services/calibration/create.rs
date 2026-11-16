//! Calibration intake: stamping a report and handing it to the calibration uploader.

// internal crates
use crate::mocks::calibration_sender::Step;
use crate::services::calibration::helpers::{params, Fixture};
use crate::test_utils::calibrations::received_at;
use miru_daemon::calibrations::{latest, CalibrationsErr, Report, ReportMetadata, UploaderOptions};
use miru_daemon::models::{DplActivity, InstanceFormat};
use miru_daemon::services::ServiceErr;

// external crates
use tokio::sync::oneshot;

#[tokio::test]
async fn queues_the_report_stamped_with_its_id_and_the_time() {
    let mut f = Fixture::new().await;

    let created = f.create(params()).await.unwrap();

    assert_eq!(created.reported_at, received_at());
    let expected = Report {
        metadata: ReportMetadata {
            report_id: created.report_id,
            component: "arm".to_string(),
            format: InstanceFormat::Yaml,
            role: Some("left".to_string()),
            aspect: Some("pose".to_string()),
            serial_number: Some("SN-1".to_string()),
            reported_by: Some("Ada Lovelace".to_string()),
            deployment_id: None,
            reported_at: received_at(),
        },
        data: params().data,
    };
    assert_eq!(f.uploaded().await, expected);
}

#[tokio::test]
async fn stamps_the_deployed_deployment() {
    let mut f = Fixture::new().await;
    f.deploy("dpl_0", DplActivity::Archived).await;
    f.deploy("dpl_1", DplActivity::Deployed).await;

    f.create(params()).await.unwrap();

    let uploaded = f.uploaded().await;
    assert_eq!(uploaded.metadata.deployment_id.as_deref(), Some("dpl_1"));
}

#[tokio::test]
async fn stamps_no_deployment_when_none_is_deployed() {
    let mut f = Fixture::new().await;
    f.deploy("dpl_0", DplActivity::Queued).await;

    f.create(params()).await.unwrap();

    assert_eq!(f.uploaded().await.metadata.deployment_id, None);
}

#[tokio::test]
async fn a_deployments_cache_error_queues_nothing() {
    let f = Fixture::new().await;
    // a report queued by mistake would stay in flight, so `num_queued_uploads` would count it
    let (_hold, hang) = oneshot::channel();
    f.sender.push(Step::Hang(hang));
    f.deployments.shutdown().await.unwrap();

    let err = f.create(params()).await.unwrap_err();

    assert!(matches!(err, ServiceErr::DiskErr(_)), "{err:?}");
    assert_eq!(f.cal_uploader.num_queued_uploads().await.unwrap(), 0);
}

#[tokio::test]
async fn a_cal_uploader_rejection_fails_intake() {
    let options = UploaderOptions {
        max_report_bytes: 100,
        ..UploaderOptions::default()
    };
    let f = Fixture::with_options(options, latest::DEFAULT_CAPACITY).await;
    let mut big = params();
    big.data = "x".repeat(200);

    let err = f.create(big).await.unwrap_err();

    assert!(
        matches!(
            err,
            ServiceErr::CalibrationsErr(CalibrationsErr::TooLargeErr(_))
        ),
        "{err:?}"
    );
}

#[tokio::test]
async fn an_oversized_report_is_too_large_even_when_the_store_is_full() {
    let options = UploaderOptions {
        max_report_bytes: 100,
        ..UploaderOptions::default()
    };
    let f = Fixture::with_options(options, 0).await;
    let mut big = params();
    big.data = "x".repeat(200);

    let err = f.create(big).await.unwrap_err();

    assert!(
        matches!(
            err,
            ServiceErr::CalibrationsErr(CalibrationsErr::TooLargeErr(_))
        ),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_full_latest_store_fails_intake() {
    let f = Fixture::with_options(UploaderOptions::default(), 0).await;
    // without the rollback the report would stay in flight, so `num_queued_uploads` would count it
    let (_hold, hang) = oneshot::channel();
    f.sender.push(Step::Hang(hang));

    let err = f.create(params()).await.unwrap_err();

    assert!(
        matches!(
            err,
            ServiceErr::CalibrationsErr(CalibrationsErr::StoreFullErr(_))
        ),
        "{err:?}"
    );
    assert_eq!(f.cal_uploader.num_queued_uploads().await.unwrap(), 0);
}
