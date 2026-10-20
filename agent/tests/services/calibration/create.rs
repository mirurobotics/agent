//! Calibration intake: stamping, buffering and waking, all offline.

// standard crates
use std::time::Duration;

// internal crates
use crate::test_utils::calibrations::layout;
use crate::test_utils::filesys::dirs as test_dirs;
use miru_agent::buffer::BufferOptions;
use miru_agent::calibrations::{self, CalibrationJob, SharedBuffer};
use miru_agent::disk::{self, Layout};
use miru_agent::errors::{Error, HTTPCode};
use miru_agent::models::{Deployment, DplActivity};
use miru_agent::services::calibration::{self, CreateParams, Created};
use miru_agent::services::ServiceErr;

// external crates
use chrono::{DateTime, Utc};
use serde_json::json;

struct Fixture {
    _dir: test_dirs::TempDir,
    buffer: SharedBuffer,
    deployments: disk::Deployments,
}

impl Fixture {
    async fn new() -> Self {
        Self::with_options(calibrations::DEFAULT_OPTIONS).await
    }

    async fn with_options(options: BufferOptions) -> Self {
        let dir = test_dirs::temp("calibration_service_test").unwrap();
        let buffer = SharedBuffer::open(&layout(&dir), options).await.unwrap();
        let layout = Layout::new(dir.to_dir());
        let (deployments, _) = disk::Deployments::spawn(64, layout.deployments(), 1000)
            .await
            .unwrap();
        Self {
            _dir: dir,
            buffer,
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
        calibration::create(&self.buffer, &self.deployments, params, now()).await
    }

    /// The buffered jobs, oldest first.
    async fn buffered(&self) -> Vec<CalibrationJob> {
        self.buffer.lock().await.jobs()
    }

    /// The oldest buffered report's content.
    async fn head_content(&self) -> serde_json::Value {
        let guard = self.buffer.lock().await;
        let entry = guard.next_ready(now()).unwrap();
        guard.read(entry.id).await.unwrap().content
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
        // extra keys are kept: the caller's value is buffered unchanged
        content: json!({ "format": "yaml", "data": "offset: 1", "note": "kept" }),
    }
}

#[tokio::test]
async fn accepted_while_offline() {
    // no HTTP client exists here: intake never talks to Miru
    let fixture = Fixture::new().await;

    let created = fixture.create(params()).await.unwrap();

    assert_eq!(created, Created { reported_at: now() });
    let expected = CalibrationJob {
        component_slug: "arm".to_string(),
        role: Some("left".to_string()),
        aspect: Some("pose".to_string()),
        serial_number: Some("SN-1".to_string()),
        deployment_id: None,
        calibrated_at: now(),
    };
    assert_eq!(fixture.buffered().await, [expected]);
    assert_eq!(fixture.head_content().await, params().content);
}

#[tokio::test]
async fn stamps_the_current_deployment() {
    let fixture = Fixture::new().await;
    fixture.deploy("dpl_0", DplActivity::Archived).await;
    fixture.deploy("dpl_1", DplActivity::Deployed).await;

    fixture.create(params()).await.unwrap();

    let buffered = fixture.buffered().await;
    assert_eq!(buffered[0].deployment_id.as_deref(), Some("dpl_1"));
}

#[tokio::test]
async fn no_deployment_is_fine() {
    let fixture = Fixture::new().await;
    fixture.deploy("dpl_0", DplActivity::Queued).await;

    fixture.create(params()).await.unwrap();

    assert_eq!(fixture.buffered().await[0].deployment_id, None);
}

#[tokio::test]
async fn a_deployments_cache_error_fails_intake() {
    let fixture = Fixture::new().await;
    fixture.deployments.shutdown().await.unwrap();

    let err = fixture.create(params()).await.unwrap_err();

    assert!(matches!(err, ServiceErr::DiskErr(_)), "{err:?}");
    assert_eq!(err.http_status(), HTTPCode::INTERNAL_SERVER_ERROR);
    assert!(fixture.buffered().await.is_empty());
}

#[tokio::test]
async fn wakes_the_worker() {
    let fixture = Fixture::new().await;

    fixture.create(params()).await.unwrap();

    tokio::time::timeout(Duration::from_secs(5), fixture.buffer.woken())
        .await
        .unwrap();
}

#[tokio::test]
async fn buffer_full_is_507() {
    let options = BufferOptions {
        max_items: 1,
        ..calibrations::DEFAULT_OPTIONS
    };
    let fixture = Fixture::with_options(options).await;
    fixture.create(params()).await.unwrap();

    let err = fixture.create(params()).await.unwrap_err();

    assert!(matches!(err, ServiceErr::CalibrationsErr(_)), "{err:?}");
    assert_eq!(err.code().as_str(), "calibration_buffer_full");
    assert_eq!(err.http_status(), HTTPCode::INSUFFICIENT_STORAGE);
    assert_eq!(fixture.buffered().await.len(), 1);
}
