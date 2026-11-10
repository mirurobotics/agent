//! The calibration service fixture: a calibration uploader with a mock sender,
//! its latest store and a deployments cache, under a temp dir.

// standard crates
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::mocks::calibration_sender::MockCalibrationSender;
use crate::test_utils::calibrations::{latest_cals_layout, latest_store, layout, received_at};
use crate::test_utils::filesys::dirs as test_dirs;
use miru_agent::calibrations::{
    latest, LatestStore, QueueSnapshotFile, Report, Uploader, UploaderArgs, UploaderOptions,
};
use miru_agent::disk::{self, Layout};
use miru_agent::filesys::{files, WriteOptions};
use miru_agent::models::{Deployment, DplActivity, InstanceFormat};
use miru_agent::services::calibration::{self, CreateParams, Created};
use miru_agent::services::ServiceErr;

// external crates
use chrono::Utc;
use tokio::sync::mpsc;

pub struct Fixture {
    dir: test_dirs::TempDir,
    pub cal_uploader: Uploader,
    /// The latest store the calibration uploader writes.
    pub latest: LatestStore,
    pub sender: Arc<MockCalibrationSender>,
    started: mpsc::UnboundedReceiver<()>,
    pub deployments: disk::Deployments,
}

impl Fixture {
    pub async fn new() -> Self {
        Self::with_options(UploaderOptions::default(), latest::DEFAULT_CAPACITY).await
    }

    /// A fixture whose calibration uploader takes `options` and whose latest store holds
    /// at most `latest_capacity` keys.
    pub async fn with_options(options: UploaderOptions, latest_capacity: usize) -> Self {
        let dir = test_dirs::temp("calibration_service_test").unwrap();
        let calibrations = layout(&dir);
        let snapshot_file =
            QueueSnapshotFile::new_with_default(calibrations.queue(), Default::default())
                .await
                .unwrap();
        let (sender, started) = MockCalibrationSender::new();
        let latest = latest_store(&dir, latest_capacity).await;
        let args = UploaderArgs {
            snapshot_file: Some(snapshot_file),
            reports: calibrations.reports(),
            latest: latest.clone(),
            sender: sender.clone(),
            options,
            sleep_fn: tokio::time::sleep,
            now_fn: Utc::now,
        };
        let (cal_uploader, _) = Uploader::spawn(64, args);
        let deployments_file = Layout::new(dir.to_dir()).deployments();
        let (deployments, _) = disk::Deployments::spawn(64, deployments_file, 1000)
            .await
            .unwrap();
        Self {
            dir,
            cal_uploader,
            latest,
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
        calibration::create(&self.cal_uploader, &self.deployments, params, received_at()).await
    }

    /// Makes the latest store's index unreadable.
    pub async fn corrupt_latest_index(&self) {
        let index = latest_cals_layout(&self.dir).index();
        files::write_string(&index, "{", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();
    }

    /// The first report the calibration uploader sends.
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
