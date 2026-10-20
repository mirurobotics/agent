// internal crates
use miru_agent::calibrations::errors::{BufferFullErr, CalibrationsErr, SerializeReportErr};
use miru_agent::errors::{Code, Error, HTTPCode};
use miru_agent::trace;

fn buffer_full() -> BufferFullErr {
    BufferFullErr {
        report_bytes: 10,
        reports: 1000,
        max_reports: 1000,
        bytes: 20,
        max_bytes: 256,
        trace: trace!(),
    }
}

#[test]
fn buffer_full_is_507_calibration_buffer_full() {
    let err = CalibrationsErr::BufferFullErr(buffer_full());
    assert!(matches!(err.code(), Code::CalibrationBufferFull));
    assert_eq!(err.code().as_str(), "calibration_buffer_full");
    assert_eq!(err.http_status(), HTTPCode::INSUFFICIENT_STORAGE);
    assert_eq!(
        err.params(),
        Some(serde_json::json!({ "max_reports": 1000, "max_bytes": 256 }))
    );
    assert_eq!(
        err.to_string(),
        "calibration buffer is full (1000/1000 reports, 20/256 bytes); rejected a report of 10 bytes"
    );
}

#[test]
fn serialize_report_err_is_internal() {
    let source = serde_json::from_str::<serde_json::Value>("{").unwrap_err();
    let err = CalibrationsErr::SerializeReportErr(SerializeReportErr {
        id: uuid::Uuid::nil(),
        source,
        trace: trace!(),
    });
    assert!(matches!(err.code(), Code::InternalServerError), "{err}");
    assert_eq!(err.http_status(), HTTPCode::INTERNAL_SERVER_ERROR);
    assert_eq!(err.params(), None);
}
