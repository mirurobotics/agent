//! Calibration reports and layouts shared by the calibration, worker and
//! service tests.

// internal crates
use crate::test_utils::filesys::dirs::TempDir;
use miru_agent::calibrations::{CalibrationJob, Report};
use miru_agent::disk::layout::CalibrationsLayout;

// external crates
use chrono::{DateTime, Utc};
use serde_json::json;

/// When every fixture report was received.
pub fn received_at() -> DateTime<Utc> {
    DateTime::from_timestamp(1000, 0).unwrap()
}

/// A sendable report told apart from others by its `component_slug`.
pub fn report(slug: &str) -> Report {
    Report {
        job: CalibrationJob {
            component_slug: slug.to_string(),
            role: None,
            aspect: None,
            serial_number: None,
            deployment_id: None,
            calibrated_at: received_at(),
        },
        content: json!({ "format": "json", "data": r#"{"offset":[1,2,3]}"# }),
    }
}

/// The calibrations layout under `dir`.
pub fn layout(dir: &TempDir) -> CalibrationsLayout {
    CalibrationsLayout::new(dir.to_dir().subdir("calibrations"))
}
