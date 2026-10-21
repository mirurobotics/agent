//! Calibration intake: stamping and buffering, all offline.

// standard crates
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::mocks::calibration_sender::{MockCalibrationSender, Step};
use crate::test_utils::calibrations::layout;
use crate::test_utils::filesys::dirs as test_dirs;
use miru_agent::calibrations::{self, Job, Report, Uploader, UploaderArgs};
use miru_agent::disk::{self, Layout};
use miru_agent::errors::{Error, HTTPCode};
use miru_agent::models::{Deployment, DplActivity};
use miru_agent::queue::DirQueueOptions;
use miru_agent::services::calibration::{self, CreateParams, Created};
use miru_agent::services::ServiceErr;

// external crates
use chrono::{DateTime, Utc};
use serde_json::json;
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
        Self::with_options(calibrations::DEFAULT_OPTIONS).await
    }

    async fn with_options(options: DirQueueOptions) -> Self {
        let dir = test_dirs::temp("calibration_service_test").unwrap();
        let calibrations_layout = layout(&dir);
        let (sender, started) = MockCalibrationSender::new();
        let args = UploaderArgs {
            queue: calibrations::open(&calibrations_layout, options)
                .await
                .unwrap(),
            failed: calibrations_layout.failed(),
            sender: sender.clone(),
            events: None,
            options: Default::default(),
            sleep_fn: tokio::time::sleep,
        };
        let (uploader, _) = Uploader::spawn(64, args);
        let layout = Layout::new(dir.to_dir());
        let (deployments, _) = disk::Deployments::spawn(64, layout.deployments(), 1000)
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
            release_id: "rel_1".to_string(),
            ..Default::default()
        };
        self.deployments
            .write_if_absent(id.to_string(), deployment, |_, _| false)
            .await
            .unwrap();
    }

    async fn create(&self, params: CreateParams) -> Result<Created, ServiceErr> {
        calibration::create(&self.uploader, &self.deployments, params, now()).await
    }

    /// The report the uploader sends first.
    async fn uploaded(&mut self) -> Report {
        tokio::time::timeout(Duration::from_secs(5), self.started.recv())
            .await
            .unwrap()
            .unwrap();
        self.sender.reports().remove(0)
    }
}

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).unwrap()
}

fn params() -> CreateParams {
    CreateParams {
        component_slug: "arm".to_string(),
        role: Some("left".to_string()),
        aspect: Some("pose".to_string()),
        serial_number: Some("SN-1".to_string()),
        // extra keys are kept: the caller's value is queued unchanged
        content: json!({ "format": "yaml", "data": "offset: 1", "note": "kept" }),
    }
}

#[tokio::test]
async fn accepted_while_offline() {
    // no HTTP client exists here: intake never talks to Miru
    let mut fixture = Fixture::new().await;

    let created = fixture.create(params()).await.unwrap();

    assert_eq!(created, Created { reported_at: now() });
    let expected = Job {
        component_slug: "arm".to_string(),
        role: Some("left".to_string()),
        aspect: Some("pose".to_string()),
        serial_number: Some("SN-1".to_string()),
        deployment_id: None,
        calibrated_at: now(),
    };
    let uploaded = fixture.uploaded().await;
    assert_eq!(uploaded.job, expected);
    assert_eq!(uploaded.content, params().content);
}

#[tokio::test]
async fn stamps_the_current_deployment() {
    let mut fixture = Fixture::new().await;
    fixture.deploy("dpl_0", DplActivity::Archived).await;
    fixture.deploy("dpl_1", DplActivity::Deployed).await;

    fixture.create(params()).await.unwrap();

    let uploaded = fixture.uploaded().await;
    assert_eq!(uploaded.job.deployment_id.as_deref(), Some("dpl_1"));
}

#[tokio::test]
async fn no_deployment_is_fine() {
    let mut fixture = Fixture::new().await;
    fixture.deploy("dpl_0", DplActivity::Queued).await;

    fixture.create(params()).await.unwrap();

    assert_eq!(fixture.uploaded().await.job.deployment_id, None);
}

#[tokio::test]
async fn a_deployments_cache_error_fails_intake() {
    let fixture = Fixture::new().await;
    fixture.deployments.shutdown().await.unwrap();

    let err = fixture.create(params()).await.unwrap_err();

    assert!(matches!(err, ServiceErr::DiskErr(_)), "{err:?}");
    assert_eq!(err.http_status(), HTTPCode::INTERNAL_SERVER_ERROR);
    assert_eq!(fixture.uploader.len().await.unwrap(), 0);
    assert!(fixture.sender.sent().is_empty());
}

#[tokio::test]
async fn queue_full_is_507() {
    let options = DirQueueOptions {
        capacity: 1,
        ..calibrations::DEFAULT_OPTIONS
    };
    let fixture = Fixture::with_options(options).await;
    // the first report stays queued while its upload hangs
    let (_hold, hang) = oneshot::channel();
    fixture.sender.push(Step::Hang(hang));
    fixture.create(params()).await.unwrap();

    let err = fixture.create(params()).await.unwrap_err();

    assert!(matches!(err, ServiceErr::CalibrationsErr(_)), "{err:?}");
    assert_eq!(err.code().as_str(), "calibration_buffer_full");
    assert_eq!(err.http_status(), HTTPCode::INSUFFICIENT_STORAGE);
    assert_eq!(fixture.uploader.len().await.unwrap(), 1);
}

#[tokio::test]
async fn an_oversized_report_is_413() {
    let options = DirQueueOptions {
        max_item_bytes: 100,
        ..calibrations::DEFAULT_OPTIONS
    };
    let fixture = Fixture::with_options(options).await;
    let mut big = params();
    big.content = json!({ "format": "text", "data": "x".repeat(200) });

    let err = fixture.create(big).await.unwrap_err();

    assert!(matches!(err, ServiceErr::CalibrationsErr(_)), "{err:?}");
    assert_eq!(err.code().as_str(), "calibration_too_large");
    assert_eq!(err.http_status(), HTTPCode::PAYLOAD_TOO_LARGE);
    assert_eq!(fixture.uploader.len().await.unwrap(), 0);
    assert!(fixture.sender.sent().is_empty());
}
