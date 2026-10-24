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
use uuid::Uuid;

/// When every fixture report was received.
pub fn received_at() -> DateTime<Utc> {
    DateTime::from_timestamp(1000, 0).unwrap()
}

/// The `report_id` of every fixture report.
pub fn report_id() -> Uuid {
    Uuid::from_u128(0x0b6f3c2e_6a51_4e0f_9a43_2f1d7c9e8b10)
}

/// A sendable report told apart from others by its `component`.
pub fn report(slug: &str) -> Report {
    Report {
        metadata: ReportMetadata {
            report_id: report_id(),
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
