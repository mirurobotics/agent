// internal crates
use crate::errors::harnesses::{assert_error, Expected};
use miru_daemon::cache::errors::CacheElementNotFound;
use miru_daemon::cache::CacheErr;
use miru_daemon::calibrations::{CalibrationTooLargeErr, CalibrationsErr};
use miru_daemon::disk::DiskErr;
use miru_daemon::errors::{Code, HTTPCode};
use miru_daemon::filesys::errors::InvalidDirNameErr;
use miru_daemon::filesys::FileSysErr;
use miru_daemon::http::errors::MockErr as HTTPMockErr;
use miru_daemon::http::HTTPErr;
use miru_daemon::models::errors::DateTimeParseErr;
use miru_daemon::models::ModelsErr;
use miru_daemon::services::errors::{
    FileRulesNotExpandedErr, InvalidReasonErr, LatestCalibrationNotFoundErr,
};
use miru_daemon::services::ServiceErr;
use miru_daemon::sync::errors::MockErr as SyncMockErr;
use miru_daemon::sync::SyncErr;

fn cache_err() -> CacheErr {
    CacheErr::CacheElementNotFound(CacheElementNotFound {
        msg: "cache miss".to_string(),
        trace: miru_daemon::trace!(),
    })
}

fn filesys_err() -> FileSysErr {
    FileSysErr::InvalidDirNameErr(InvalidDirNameErr {
        name: "bad/dir".to_string(),
        trace: miru_daemon::trace!(),
    })
}

fn models_err() -> ModelsErr {
    let source = chrono::DateTime::parse_from_rfc3339("invalid_datetime").unwrap_err();
    ModelsErr::DateTimeParseErr(DateTimeParseErr {
        source,
        trace: miru_daemon::trace!(),
    })
}

fn storage_err() -> DiskErr {
    DiskErr::CacheErr(cache_err())
}

fn http_err() -> HTTPErr {
    HTTPErr::MockErr(HTTPMockErr {
        is_network_conn_err: false,
    })
}

fn sync_err() -> SyncErr {
    SyncErr::MockErr(SyncMockErr {
        is_network_conn_err: false,
    })
}

fn calibrations_err() -> CalibrationsErr {
    CalibrationsErr::TooLargeErr(CalibrationTooLargeErr {
        report_bytes: 200,
        max_bytes: 100,
        trace: miru_daemon::trace!(),
    })
}

fn file_rules_not_expanded_err() -> FileRulesNotExpandedErr {
    FileRulesNotExpandedErr {
        release_id: "rls_123".to_string(),
    }
}

fn invalid_reason_err() -> InvalidReasonErr {
    InvalidReasonErr {
        max_chars: 512,
        trace: miru_daemon::trace!(),
    }
}

fn latest_calibration_not_found_err() -> LatestCalibrationNotFoundErr {
    LatestCalibrationNotFoundErr {
        component: "arm".to_string(),
        trace: miru_daemon::trace!(),
    }
}

#[test]
fn an_invalid_reason_is_422() {
    let err = ServiceErr::from(invalid_reason_err());

    assert_error(
        &err,
        Expected::new(Code::InvalidRequest, HTTPCode::UNPROCESSABLE_ENTITY),
    );
    let want = "an invalidation reason must not be blank or over 512 characters";
    assert_eq!(err.to_string(), want);
}

#[test]
fn a_component_without_a_matching_latest_calibration_is_404() {
    let err = ServiceErr::from(latest_calibration_not_found_err());

    assert_error(
        &err,
        Expected::new(Code::ResourceNotFound, HTTPCode::NOT_FOUND),
    );
    assert_eq!(err.to_string(), "no latest calibration of arm matches");
}

mod from_conversions {
    use super::*;

    #[test]
    fn cache_err_maps_to_service_cache_err() {
        let err: ServiceErr = cache_err().into();
        assert!(matches!(err, ServiceErr::CacheErr(_)));
    }

    #[test]
    fn filesys_err_maps_to_service_filesys_err() {
        let err: ServiceErr = filesys_err().into();
        assert!(matches!(err, ServiceErr::FileSysErr(_)));
    }

    #[test]
    fn models_err_maps_to_service_models_err() {
        let err: ServiceErr = models_err().into();
        assert!(matches!(err, ServiceErr::ModelsErr(_)));
    }

    #[test]
    fn storage_err_maps_to_service_storage_err() {
        let err: ServiceErr = storage_err().into();
        assert!(matches!(err, ServiceErr::DiskErr(_)));
    }

    #[test]
    fn http_err_maps_to_service_http_err() {
        let err: ServiceErr = http_err().into();
        assert!(matches!(err, ServiceErr::HTTPErr(_)));
    }

    #[test]
    fn sync_err_maps_to_service_sync_err() {
        let err: ServiceErr = sync_err().into();
        assert!(matches!(err, ServiceErr::SyncErr(_)));
    }

    #[test]
    fn file_rules_not_expanded_err_maps_to_service_variant() {
        let err: ServiceErr = file_rules_not_expanded_err().into();
        assert!(matches!(err, ServiceErr::FileRulesNotExpanded(_)));
    }

    #[test]
    fn calibrations_err_maps_to_service_calibrations_err() {
        let err: ServiceErr = calibrations_err().into();
        assert!(matches!(err, ServiceErr::CalibrationsErr(_)));
    }

    #[test]
    fn invalid_reason_err_maps_to_service_invalid_reason_err() {
        let err: ServiceErr = invalid_reason_err().into();
        assert!(matches!(err, ServiceErr::InvalidReasonErr(_)));
    }

    #[test]
    fn latest_calibration_not_found_err_maps_to_its_service_variant() {
        let err: ServiceErr = latest_calibration_not_found_err().into();
        assert!(matches!(err, ServiceErr::LatestCalibrationNotFoundErr(_)));
    }
}
