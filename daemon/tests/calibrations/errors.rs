// internal crates
use crate::errors::harnesses::{assert_error, Expected};
use miru_daemon::calibrations::{
    CalibrationBufferFullErr, CalibrationNotFoundErr, CalibrationStoreFullErr,
    CalibrationTooLargeErr, CalibrationsErr, LatestCalibrationNotFoundErr, ReportNotRecordedErr,
};
use miru_daemon::errors::{Code, Error, HTTPCode};
use miru_daemon::http::errors::RequestFailed;
use miru_daemon::http::request::Params;
use miru_daemon::queue::QueueFullErr;

// external crates
use serde_json::json;
use uuid::Uuid;

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
        trace: miru_daemon::trace!(),
    };

    let want = Expected::new(Code::CalibrationTooLarge, HTTPCode::PAYLOAD_TOO_LARGE)
        .with_params(json!({ "max_bytes": 100 }));
    assert_error(&err, want);
}

#[test]
fn a_full_store_is_507_with_its_cap() {
    let err = CalibrationsErr::from(CalibrationStoreFullErr {
        capacity: 1000,
        trace: miru_daemon::trace!(),
    });

    let want = Expected::new(Code::CalibrationStoreFull, HTTPCode::INSUFFICIENT_STORAGE)
        .with_params(json!({ "max_keys": 1000 }));
    assert_error(&err, want);
    assert!(!err.is_terminal());
}

#[test]
fn a_missing_latest_calibration_is_404() {
    let err = CalibrationsErr::from(CalibrationNotFoundErr {
        report_id: "not-a-report".to_string(),
        trace: miru_daemon::trace!(),
    });

    assert_error(
        &err,
        Expected::new(Code::ResourceNotFound, HTTPCode::NOT_FOUND),
    );
    assert!(!err.is_terminal());
}

#[test]
fn a_component_without_a_matching_latest_calibration_is_404() {
    let err = CalibrationsErr::from(LatestCalibrationNotFoundErr {
        component: "arm".to_string(),
        report_id: Some(Uuid::from_u128(1)),
        trace: miru_daemon::trace!(),
    });

    assert_error(
        &err,
        Expected::new(Code::ResourceNotFound, HTTPCode::NOT_FOUND),
    );
    assert!(!err.is_terminal());
}

#[test]
fn an_unrecorded_report_is_retried() {
    let err = CalibrationsErr::from(ReportNotRecordedErr {
        report_id: Uuid::from_u128(1),
        source: RequestFailed {
            request: Params::get("http://mock/calibrations").meta().unwrap(),
            status: reqwest::StatusCode::NOT_FOUND,
            error: None,
            trace: miru_daemon::trace!(),
        },
    });

    assert_error(
        &err,
        Expected::new(Code::InternalServerError, HTTPCode::INTERNAL_SERVER_ERROR),
    );
    assert!(!err.is_terminal());
}
