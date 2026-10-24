//! Calibration reports, layouts and uploaders shared by the calibration,
//! worker, service and server tests.

// standard crates
use std::sync::Arc;

// internal crates
use super::filesys::dirs::TempDir;
use miru_agent::calibrations::{
    Report, ReportMetadata, Sender, Uploader, UploaderArgs, UploaderOptions,
};
use miru_agent::disk::layout::CalibrationsLayout;
use miru_agent::models::InstanceFormat;

// external crates
use chrono::{DateTime, Utc};

/// When every fixture report was received.
pub fn received_at() -> DateTime<Utc> {
    DateTime::from_timestamp(1000, 0).unwrap()
}

/// A sendable report told apart from others by its `component`.
pub fn report(slug: &str) -> Report {
    Report {
        metadata: ReportMetadata {
            component: slug.to_string(),
            format: InstanceFormat::Json,
            role: None,
            aspect: None,
            serial_number: None,
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

/// An uploader under `dir` that keeps its queue in memory and uploads through
/// `sender`.
pub fn uploader<S: Sender + 'static>(
    dir: &TempDir,
    sender: Arc<S>,
    options: UploaderOptions,
) -> Uploader {
    let args = UploaderArgs {
        snapshot_file: None,
        reports: layout(dir).reports(),
        sender,
        options,
        sleep_fn: tokio::time::sleep,
        now_fn: Utc::now,
    };
    Uploader::spawn(64, args).0
}
