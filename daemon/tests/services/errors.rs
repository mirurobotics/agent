// internal crates
use miru_daemon::cache::errors::CacheElementNotFound;
use miru_daemon::cache::CacheErr;
use miru_daemon::calibrations::{CalibrationTooLargeErr, CalibrationsErr};
use miru_daemon::disk::DiskErr;
use miru_daemon::filesys::errors::InvalidDirNameErr;
use miru_daemon::filesys::FileSysErr;
use miru_daemon::http::errors::MockErr as HTTPMockErr;
use miru_daemon::http::HTTPErr;
use miru_daemon::models::errors::DateTimeParseErr;
use miru_daemon::models::ModelsErr;
use miru_daemon::services::errors::FileRulesNotExpandedErr;
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
}
