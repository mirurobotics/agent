//! Calibration intake: stamping a report and handing it to the uploader.

// standard crates
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::mocks::calibration_sender::{MockCalibrationSender, Step};
use crate::test_utils::calibrations::{layout, received_at};
use crate::test_utils::filesys::dirs as test_dirs;
use miru_agent::calibrations::{
    CalibrationsErr, QueueSnapshotFile, Report, ReportMetadata, Uploader, UploaderArgs,
    UploaderOptions,
};
use miru_agent::disk::{self, Layout};
use miru_agent::models::{Deployment, DplActivity, InstanceFormat};
use miru_agent::services::calibration::{self, CreateParams, Created};
use miru_agent::services::ServiceErr;

// external crates
use chrono::Utc;
use tokio::sync::{mpsc, oneshot};

struct Fixture {
    _dir: test_dirs::TempDir,
    uploader: Uploader,
    sender: Arc<MockCalibrationSender>,
    started: mpsc::UnboundedReceiver<()>,
    deployments: disk::Deployments,
}

impl Fixture {
    async fn new() -> Self {
        Self::with_options(UploaderOptions::default()).await
    }

    async fn with_options(options: UploaderOptions) -> Self {
        let dir = test_dirs::temp("calibration_service_test").unwrap();
        let calibrations = layout(&dir);
        let snapshot_file =
            QueueSnapshotFile::new_with_default(calibrations.queue(), Default::default())
                .await
                .unwrap();
        let (sender, started) = MockCalibrationSender::new();
        let args = UploaderArgs {
            snapshot_file: Some(snapshot_file),
            reports: calibrations.reports(),
            sender: sender.clone(),
            options,
            sleep_fn: tokio::time::sleep,
            now_fn: Utc::now,
        };
        let (uploader, _) = Uploader::spawn(64, args);
        let deployments_file = Layout::new(dir.to_dir()).deployments();
        let (deployments, _) = disk::Deployments::spawn(64, deployments_file, 1000)
            .await
            .unwrap();
        Self {
            _dir: dir,
            uploader,
            sender,
            started,
            deployments,
        }
    }

    async fn deploy(&self, id: &str, activity_status: DplActivity) {
        let deployment = Deployment {
            id: id.to_string(),
            activity_status,
            ..Default::default()
        };
        self.deployments
            .write_if_absent(id.to_string(), deployment, |_, _| false)
            .await
            .unwrap();
    }

    async fn create(&self, params: CreateParams) -> Result<Created, ServiceErr> {
        calibration::create(&self.uploader, &self.deployments, params, received_at()).await
    }

    /// The first report the uploader sends.
    async fn uploaded(&mut self) -> Report {
        tokio::time::timeout(Duration::from_secs(60), self.started.recv())
            .await
            .unwrap()
            .unwrap();
        self.sender.reports().remove(0)
    }
}

fn params() -> CreateParams {
    CreateParams {
        component: "arm".to_string(),
        role: Some("left".to_string()),
        aspect: Some("pose".to_string()),
        serial_number: Some("SN-1".to_string()),
        format: InstanceFormat::Yaml,
        // the caller's data is queued unchanged, comments and all
        data: "offset: 1 # kept\n".to_string(),
    }
}

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
    // a report queued by mistake would stay in flight, so `len` would count it
    let (_hold, hang) = oneshot::channel();
    f.sender.push(Step::Hang(hang));
    f.deployments.shutdown().await.unwrap();

    let err = f.create(params()).await.unwrap_err();

    assert!(matches!(err, ServiceErr::DiskErr(_)), "{err:?}");
    assert_eq!(f.uploader.len().await.unwrap(), 0);
}

#[tokio::test]
async fn an_uploader_rejection_fails_intake() {
    let options = UploaderOptions {
        max_report_bytes: 100,
        ..UploaderOptions::default()
    };
    let f = Fixture::with_options(options).await;
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
