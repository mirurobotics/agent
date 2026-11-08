// internal crates
use crate::errors::harnesses::{assert_error, Expected};
use miru_agent::cache::{errors::CacheElementNotFound, CacheErr};
use miru_agent::calibrations::{
    CalibrationBufferFullErr, CalibrationNotFoundErr, CalibrationStoreFullErr,
    CalibrationTooLargeErr, CalibrationsErr,
};
use miru_agent::errors::{Code, Error, HTTPCode};
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

#[test]
fn a_full_store_is_507_with_its_cap() {
    let err = CalibrationsErr::from(CalibrationStoreFullErr {
        capacity: 1000,
        trace: miru_agent::trace!(),
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
        trace: miru_agent::trace!(),
    });

    assert_error(
        &err,
        Expected::new(Code::ResourceNotFound, HTTPCode::NOT_FOUND),
    );
    assert!(!err.is_terminal());
}

/// A latest store's cache failure keeps the cache error's code and status.
#[test]
fn a_cache_error_keeps_its_code() {
    let missing = CacheElementNotFound {
        msg: "missing".to_string(),
        trace: miru_agent::trace!(),
    };
    let err = CalibrationsErr::from(CacheErr::CacheElementNotFound(missing));

    assert_error(
        &err,
        Expected::new(Code::ResourceNotFound, HTTPCode::NOT_FOUND),
    );
    assert!(!err.is_terminal());
}
