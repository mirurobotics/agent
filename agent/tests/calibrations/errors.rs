// internal crates
use crate::errors::harnesses::{assert_error, Expected};
use miru_agent::calibrations::{CalibrationBufferFullErr, CalibrationTooLargeErr};
use miru_agent::errors::{Code, HTTPCode};
use miru_agent::queue::QueueFullErr;

// external crates
use serde_json::json;

#[test]
fn a_full_buffer_is_507_with_its_cap() {
    let err = CalibrationBufferFullErr(QueueFullErr::new("calibration", 1000, "report"));

    let want = Expected::new(Code::CalibrationBufferFull, HTTPCode::INSUFFICIENT_STORAGE)
        .with_params(json!({ "max_reports": 1000 }));
    assert_error(&err, want);
}

#[test]
fn an_oversized_report_is_413_with_its_cap() {
    let err = CalibrationTooLargeErr {
        report_bytes: 200,
        max_bytes: 100,
        trace: miru_agent::trace!(),
    };

    let want = Expected::new(Code::CalibrationTooLarge, HTTPCode::PAYLOAD_TOO_LARGE)
        .with_params(json!({ "max_bytes": 100 }));
    assert_error(&err, want);
}
