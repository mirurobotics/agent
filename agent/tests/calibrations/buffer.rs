//! The calibration instantiation of the generic buffer: its paths, caps and
//! the 507 it maps a full buffer to. Buffer behavior itself is covered in
//! `agent/tests/buffer/`.

// internal crates
use crate::test_utils::filesys::dirs as test_dirs;
use miru_agent::buffer::BufferOptions;
use miru_agent::calibrations::{self, CalibrationJob, CalibrationsErr, Report};
use miru_agent::disk::layout::CalibrationsLayout;
use miru_agent::errors::{Code, Error, HTTPCode};
use miru_agent::filesys::PathExt;

// external crates
use chrono::DateTime;
use serde_json::json;

fn report(slug: &str) -> Report {
    Report {
        job: CalibrationJob {
            component_slug: slug.to_string(),
            role: None,
            aspect: None,
            serial_number: None,
            deployment_id: None,
            calibrated_at: DateTime::from_timestamp(1000, 0).unwrap(),
        },
        content: json!({ "offset": [1, 2, 3] }),
    }
}

fn layout(dir: &test_dirs::TempDir) -> CalibrationsLayout {
    CalibrationsLayout::new(dir.to_dir().subdir("calibrations"))
}

#[test]
fn default_options_are_1000_reports_or_256_mib() {
    assert_eq!(calibrations::DEFAULT_OPTIONS.max_items, 1000);
    assert_eq!(calibrations::DEFAULT_OPTIONS.max_bytes, 256 * 1024 * 1024);
}

#[tokio::test]
async fn stores_reports_under_the_calibrations_layout() {
    let dir = test_dirs::temp("calibration_buffer_test").unwrap();
    let layout = layout(&dir);

    let id = {
        let mut buffer = calibrations::open(&layout, calibrations::DEFAULT_OPTIONS)
            .await
            .unwrap();
        buffer.enqueue(report("arm")).await.unwrap()
    };

    assert!(layout.queue().exists());
    assert!(layout.content().file(&format!("{id}.json")).exists());
    let buffer = calibrations::open(&layout, calibrations::DEFAULT_OPTIONS)
        .await
        .unwrap();
    assert_eq!(buffer.read(id).await.unwrap(), report("arm"));
}

#[tokio::test]
async fn a_full_buffer_is_507_calibration_buffer_full() {
    let dir = test_dirs::temp("calibration_buffer_test").unwrap();
    let options = BufferOptions {
        max_items: 1,
        max_bytes: 1024,
    };
    let mut buffer = calibrations::open(&layout(&dir), options).await.unwrap();
    buffer.enqueue(report("arm")).await.unwrap();

    let err = CalibrationsErr::from(buffer.enqueue(report("leg")).await.unwrap_err());

    assert!(matches!(err, CalibrationsErr::BufferFullErr(_)), "{err:?}");
    assert!(matches!(err.code(), Code::CalibrationBufferFull));
    assert_eq!(err.code().as_str(), "calibration_buffer_full");
    assert_eq!(err.http_status(), HTTPCode::INSUFFICIENT_STORAGE);
    assert_eq!(
        err.params(),
        Some(json!({ "max_reports": 1, "max_bytes": 1024 }))
    );
}

#[tokio::test]
async fn other_buffer_errors_pass_through_as_internal() {
    let dir = test_dirs::temp("calibration_buffer_test").unwrap();
    let layout = layout(&dir);
    // a file where the buffer root should be
    std::fs::write(layout.root.path(), "x").unwrap();

    let Err(err) = calibrations::open(&layout, calibrations::DEFAULT_OPTIONS).await else {
        panic!("expected the open to fail");
    };

    assert!(matches!(err, CalibrationsErr::BufferErr(_)), "{err:?}");
    assert!(matches!(err.code(), Code::InternalServerError), "{err}");
    assert_eq!(err.http_status(), HTTPCode::INTERNAL_SERVER_ERROR);
}
