// internal crates
use miru_agent::calibrations::errors::{
    BufferFullErr, CalibrationsErr, DuplicateReportErr, InvalidReportIdErr,
};
use miru_agent::errors::{Code, Error, HTTPCode};
use miru_agent::trace;

fn buffer_full() -> BufferFullErr {
    BufferFullErr {
        report_id: "clb_rpt_a".to_string(),
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
        "calibration buffer is full (1000/1000 reports, 20/256 bytes); rejected report clb_rpt_a of 10 bytes"
    );
}

#[test]
fn report_id_errors_are_internal() {
    let errs = [
        CalibrationsErr::InvalidReportIdErr(InvalidReportIdErr {
            report_id: "../x".to_string(),
            trace: trace!(),
        }),
        CalibrationsErr::DuplicateReportErr(DuplicateReportErr {
            report_id: "clb_rpt_a".to_string(),
            trace: trace!(),
        }),
    ];
    for err in errs {
        assert!(matches!(err.code(), Code::InternalServerError), "{err}");
        assert_eq!(err.http_status(), HTTPCode::INTERNAL_SERVER_ERROR, "{err}");
        assert_eq!(err.params(), None, "{err}");
    }
}
