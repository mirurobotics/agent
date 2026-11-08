//! The calibration service fixture: a calibration manager with a mock sender
//! and a deployments cache, under a temp dir.

// standard crates
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::mocks::calibration_sender::MockCalibrationSender;
use crate::test_utils::calibrations::{latest_layout, received_at, reports_layout};
use crate::test_utils::filesys::dirs as test_dirs;
use miru_agent::calibrations::{
    latest::DEFAULT_CAPACITY, CalMngrArgs, CalibrationManager, LatestStore, QueueSnapshotFile,
    Report, UploaderOptions,
};
use miru_agent::disk::{self, Layout};
use miru_agent::models::{Deployment, DplActivity, InstanceFormat};
use miru_agent::services::calibration::{self, CreateParams, Created};
use miru_agent::services::ServiceErr;

// external crates
use chrono::Utc;
use tokio::sync::mpsc;

pub struct Fixture {
    _dir: test_dirs::TempDir,
    pub cal_mngr: CalibrationManager,
    pub sender: Arc<MockCalibrationSender>,
    started: mpsc::UnboundedReceiver<()>,
    pub deployments: disk::Deployments,
}

impl Fixture {
    pub async fn new() -> Self {
        Self::with_options(UploaderOptions::default(), DEFAULT_CAPACITY).await
    }

    /// A fixture whose calibration manager takes `options` and whose latest store holds
    /// at most `latest_capacity` keys.
    pub async fn with_options(options: UploaderOptions, latest_capacity: usize) -> Self {
        let dir = test_dirs::temp("calibration_service_test").unwrap();
        let calibrations = reports_layout(&dir);
        let snapshot_file =
            QueueSnapshotFile::new_with_default(calibrations.queue(), Default::default())
                .await
                .unwrap();
        let (sender, started) = MockCalibrationSender::new();
        let args = CalMngrArgs {
            snapshot_file: Some(snapshot_file),
            reports: calibrations.reports(),
            latest: LatestStore::new(None, latest_layout(&dir).contents(), latest_capacity),
            sender: sender.clone(),
            options,
            sleep_fn: tokio::time::sleep,
            now_fn: Utc::now,
        };
        let (cal_mngr, _) = CalibrationManager::spawn(64, args);
        let deployments_file = Layout::new(dir.to_dir()).deployments();
        let (deployments, _) = disk::Deployments::spawn(64, deployments_file, 1000)
            .await
            .unwrap();
        Self {
            _dir: dir,
            cal_mngr,
            sender,
            started,
            deployments,
        }
    }

    pub async fn deploy(&self, id: &str, activity_status: DplActivity) {
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

    pub async fn create(&self, params: CreateParams) -> Result<Created, ServiceErr> {
        calibration::create(&self.cal_mngr, &self.deployments, params, received_at()).await
    }

    /// The first report the calibration manager sends.
    pub async fn uploaded(&mut self) -> Report {
        tokio::time::timeout(Duration::from_secs(60), self.started.recv())
            .await
            .unwrap()
            .unwrap();
        self.sender.reports().remove(0)
    }
}

pub fn params() -> CreateParams {
    CreateParams {
        component: "arm".to_string(),
        role: Some("left".to_string()),
        aspect: Some("pose".to_string()),
        serial_number: Some("SN-1".to_string()),
        reported_by: Some("Ada Lovelace".to_string()),
        format: InstanceFormat::Yaml,
        // the caller's data is queued unchanged, comments and all
        data: "offset: 1 # kept\n".to_string(),
    }
}
