// internal crates
use crate::errors::harnesses::{assert_error, Expected};
use miru_daemon::authn::errors::MockError as AuthnMockError;
use miru_daemon::authn::AuthnErr;
use miru_daemon::cache::errors::CacheElementNotFound;
use miru_daemon::cache::CacheErr;
use miru_daemon::calibrations::{CalibrationTooLargeErr, CalibrationsErr};
use miru_daemon::crypt::errors::InvalidJWTErr;
use miru_daemon::crypt::CryptErr;
use miru_daemon::data_uploads::scan::errors::InternalError as ScanInternalError;
use miru_daemon::data_uploads::scan::ScanErr;
use miru_daemon::disk::DiskErr;
use miru_daemon::errors::{Code, HTTPCode};
use miru_daemon::events::errors::{EventsErr, MalformedCursorErr};
use miru_daemon::filesys::errors::InvalidDirNameErr;
use miru_daemon::filesys::FileSysErr;
use miru_daemon::http::errors::MockErr as HTTPMockErr;
use miru_daemon::http::HTTPErr;
use miru_daemon::server::errors::{CalibrationRequestTooLargeErr, InvalidRequestErr};
use miru_daemon::server::ServerErr;
use miru_daemon::services::ServiceErr;
use miru_daemon::sync::errors::MockErr as SyncMockErr;
use miru_daemon::sync::SyncErr;

// external crates
use serde_json::json;

fn authn_err() -> AuthnErr {
    AuthnErr::MockError(AuthnMockError {
        is_network_conn_err: false,
        trace: miru_daemon::trace!(),
    })
}

fn cache_err() -> CacheErr {
    CacheErr::CacheElementNotFound(CacheElementNotFound {
        msg: "cache miss".to_string(),
        trace: miru_daemon::trace!(),
    })
}

fn calibrations_err() -> CalibrationsErr {
    CalibrationsErr::TooLargeErr(CalibrationTooLargeErr {
        report_bytes: 200,
        max_bytes: 100,
        trace: miru_daemon::trace!(),
    })
}

fn crypt_err() -> CryptErr {
    CryptErr::InvalidJWTErr(InvalidJWTErr {
        msg: "invalid jwt".to_string(),
        trace: miru_daemon::trace!(),
    })
}

fn filesys_err() -> FileSysErr {
    FileSysErr::InvalidDirNameErr(InvalidDirNameErr {
        name: "bad/dir".to_string(),
        trace: miru_daemon::trace!(),
    })
}

fn http_err() -> HTTPErr {
    HTTPErr::MockErr(HTTPMockErr {
        is_network_conn_err: false,
    })
}

fn service_err() -> ServiceErr {
    ServiceErr::SyncErr(SyncErr::MockErr(SyncMockErr {
        is_network_conn_err: false,
    }))
}

fn storage_err() -> DiskErr {
    DiskErr::CacheErr(cache_err())
}

fn events_err() -> EventsErr {
    EventsErr::MalformedCursorErr(MalformedCursorErr {
        trace: miru_daemon::trace!(),
    })
}

fn scan_err() -> ScanErr {
    ScanErr::InternalError(ScanInternalError {
        message: "scan failure".to_string(),
        trace: miru_daemon::trace!(),
    })
}

fn sync_err() -> SyncErr {
    SyncErr::MockErr(SyncMockErr {
        is_network_conn_err: false,
    })
}

mod from_conversions {
    use super::*;

    #[test]
    fn authn_err_maps_to_server_authn_err() {
        let err: ServerErr = authn_err().into();
        assert!(matches!(err, ServerErr::AuthnErr(_)));
    }

    #[test]
    fn cache_err_maps_to_server_cache_err() {
        let err: ServerErr = cache_err().into();
        assert!(matches!(err, ServerErr::CacheErr(_)));
    }

    #[test]
    fn calibrations_err_maps_to_server_calibrations_err() {
        let err: ServerErr = calibrations_err().into();
        assert!(matches!(err, ServerErr::CalibrationsErr(_)));
    }

    #[test]
    fn crypt_err_maps_to_server_crypt_err() {
        let err: ServerErr = crypt_err().into();
        assert!(matches!(err, ServerErr::CryptErr(_)));
    }

    #[test]
    fn filesys_err_maps_to_server_filesys_err() {
        let err: ServerErr = filesys_err().into();
        assert!(matches!(err, ServerErr::FileSysErr(_)));
    }

    #[test]
    fn http_err_maps_to_server_http_err() {
        let err: ServerErr = http_err().into();
        assert!(matches!(err, ServerErr::HTTPErr(_)));
    }

    #[test]
    fn service_err_maps_to_server_service_err() {
        let err: ServerErr = service_err().into();
        assert!(matches!(err, ServerErr::ServiceErr(_)));
    }

    #[test]
    fn storage_err_maps_to_server_storage_err() {
        let err: ServerErr = storage_err().into();
        assert!(matches!(err, ServerErr::DiskErr(_)));
    }

    #[test]
    fn events_err_maps_to_server_events_err() {
        let err: ServerErr = events_err().into();
        assert!(matches!(err, ServerErr::EventsErr(_)));
    }

    #[test]
    fn scan_err_maps_to_server_scan_err() {
        let err: ServerErr = scan_err().into();
        assert!(matches!(err, ServerErr::ScanErr(_)));
    }

    #[test]
    fn sync_err_maps_to_server_sync_err() {
        let err: ServerErr = sync_err().into();
        assert!(matches!(err, ServerErr::SyncErr(_)));
    }
}

mod request_errors {
    use super::*;

    #[test]
    fn an_invalid_request_is_422() {
        let err = InvalidRequestErr {
            msg: "missing field".to_string(),
            trace: miru_daemon::trace!(),
        };

        assert_error(
            &err,
            Expected::new(Code::InvalidRequest, HTTPCode::UNPROCESSABLE_ENTITY),
        );
    }

    #[test]
    fn an_oversized_calibration_request_is_413_with_its_cap() {
        let err = CalibrationRequestTooLargeErr {
            max_request_bytes: 100,
            trace: miru_daemon::trace!(),
        };

        let want = Expected::new(Code::CalibrationTooLarge, HTTPCode::PAYLOAD_TOO_LARGE)
            .with_params(json!({ "max_request_bytes": 100 }));
        assert_error(&err, want);
    }
}
