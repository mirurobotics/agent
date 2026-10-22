//! Calibration reports and layouts shared by the calibration, worker and
//! service tests.

// internal crates
use crate::test_utils::filesys::dirs::TempDir;
use miru_agent::calibrations::{Report, ReportMetadata};
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
            calibrated_at: received_at(),
        },
        data: r#"{"offset":[1,2,3]}"#.to_string(),
    }
}

/// The calibrations layout under `dir`.
pub fn layout(dir: &TempDir) -> CalibrationsLayout {
    CalibrationsLayout::new(dir.to_dir().subdir("calibrations"))
}
