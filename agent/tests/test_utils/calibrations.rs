//! Calibration reports, layouts and uploaders shared by the calibration,
//! worker, service and server tests.

// standard crates
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

// internal crates
use super::filesys::dirs::TempDir;
use miru_agent::calibrations::{
    CalMngrArgs, CalibrationManager, LatestStore, Report, ReportMetadata, Sender, UploaderOptions,
};
use miru_agent::disk::layout::{CalibrationsLayout, LatestCalsLayout};
use miru_agent::disk::{LatestCalContent, LatestCalStor, LatestCals};
use miru_agent::models::InstanceFormat;

// external crates
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// When every fixture report was received.
pub fn received_at() -> DateTime<Utc> {
    DateTime::from_timestamp(1000, 0).unwrap()
}

/// The `report_id` of the fixture report for `slug`, the same on every call.
pub fn report_id(slug: &str) -> Uuid {
    let mut hasher = DefaultHasher::new();
    slug.hash(&mut hasher);
    Uuid::from_u64_pair(0, hasher.finish())
}

/// A sendable report told apart from others by its `component`.
pub fn report(slug: &str) -> Report {
    Report {
        metadata: ReportMetadata {
            report_id: report_id(slug),
            component: slug.to_string(),
            format: InstanceFormat::Json,
            role: None,
            aspect: None,
            serial_number: None,
            reported_by: None,
            deployment_id: None,
            reported_at: received_at(),
        },
        data: r#"{"offset":[1,2,3]}"#.to_string(),
    }
}

/// The calibrations layout under `dir`.
pub fn layout(dir: &TempDir) -> CalibrationsLayout {
    CalibrationsLayout::new(dir.to_dir().subdir("calibrations"))
}

/// The latest calibrations layout under `dir`.
pub fn latest_cals_layout(dir: &TempDir) -> LatestCalsLayout {
    let resources = dir.to_dir().subdir("resources");
    LatestCalsLayout::new(resources.subdir("latest_calibrations"))
}

/// The latest calibration caches under `dir`, holding at most `capacity` keys.
pub async fn latest_cal_stor(dir: &TempDir, capacity: usize) -> LatestCalStor {
    let layout = latest_cals_layout(dir);
    let (meta, _) = LatestCals::spawn(64, layout.index(), capacity)
        .await
        .unwrap();
    let (content, _) = LatestCalContent::spawn(64, layout.contents(), capacity)
        .await
        .unwrap();
    LatestCalStor {
        meta: Arc::new(meta),
        content: Arc::new(content),
    }
}

/// A latest store under `dir` holding at most `capacity` keys.
pub async fn latest_store(dir: &TempDir, capacity: usize) -> LatestStore {
    LatestStore::new(latest_cal_stor(dir, capacity).await)
}

/// A calibration manager under `dir` that keeps its queue in memory, holds at
/// most `latest_capacity` keys in its latest store, and uploads through
/// `sender`.
pub async fn cal_mngr<S: Sender + 'static>(
    dir: &TempDir,
    sender: Arc<S>,
    options: UploaderOptions,
    latest_capacity: usize,
) -> CalibrationManager {
    let args = CalMngrArgs {
        snapshot_file: None,
        reports: layout(dir).reports(),
        latest: latest_store(dir, latest_capacity).await,
        sender,
        options,
        sleep_fn: tokio::time::sleep,
        now_fn: Utc::now,
    };
    CalibrationManager::spawn(64, args).0
}
