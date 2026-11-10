//! Calibration reports, layouts and uploaders shared by the calibration,
//! worker, service and server tests.

// standard crates
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

// internal crates
use super::filesys::dirs::TempDir;
use miru_agent::calibrations::{
    Filter, LatestStore, Report, ReportMetadata, Sender, Uploader, UploaderArgs, UploaderOptions,
};
use miru_agent::disk::layout::{CalibrationsLayout, LatestCalsLayout};
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

/// A fixture report's metadata for the key (`component`, `role`, `aspect`),
/// with its own `report_id`.
pub fn reported(component: &str, role: Option<&str>, aspect: Option<&str>) -> ReportMetadata {
    ReportMetadata {
        report_id: Uuid::new_v4(),
        role: role.map(str::to_string),
        aspect: aspect.map(str::to_string),
        ..report(component).metadata
    }
}

/// The calibrations layout under `dir`.
pub fn layout(dir: &TempDir) -> CalibrationsLayout {
    CalibrationsLayout::new(dir.to_dir().subdir("calibrations"))
}

/// The latest calibrations layout under `dir`.
pub fn latest_cals_layout(dir: &TempDir) -> LatestCalsLayout {
    layout(dir).latest()
}

/// A latest store under `dir` holding at most `capacity` keys, once its worker
/// has pruned the content no entry names.
pub async fn latest_store(dir: &TempDir, capacity: usize) -> LatestStore {
    let (store, _) = LatestStore::spawn(64, latest_cals_layout(dir), capacity);
    // a reply means the prune ran, since commands queue behind it; an
    // unreadable index fails the reply, so its result is ignored
    let _ = store.list(&Filter::default()).await;
    store
}

/// A calibration uploader under `dir` that keeps its queue in memory, writes
/// through to `latest`, and uploads through `sender`.
pub fn cal_uploader<S: Sender + 'static>(
    dir: &TempDir,
    sender: Arc<S>,
    options: UploaderOptions,
    latest: LatestStore,
) -> Uploader {
    let args = UploaderArgs {
        snapshot_file: None,
        reports: layout(dir).reports(),
        latest,
        sender,
        options,
        sleep_fn: tokio::time::sleep,
        now_fn: Utc::now,
    };
    Uploader::spawn(64, args).0
}
