use miru_agent::cache::errors::CacheElementNotFound;
use miru_agent::cache::CacheErr;
use miru_agent::filesys::errors::InvalidDirNameErr;
use miru_agent::filesys::FileSysErr;
use miru_agent::http::errors::MockErr as HTTPMockErr;
use miru_agent::http::HTTPErr;
use miru_agent::models::errors::DateTimeParseErr;
use miru_agent::models::ModelsErr;
use miru_agent::services::ServiceErr;
use miru_agent::storage::StorageErr;
use miru_agent::sync::errors::MockErr as SyncMockErr;
use miru_agent::sync::SyncErr;

fn cache_err() -> CacheErr {
    CacheErr::CacheElementNotFound(CacheElementNotFound {
        msg: "cache miss".to_string(),
        trace: miru_agent::trace!(),
    })
}

fn filesys_err() -> FileSysErr {
    FileSysErr::InvalidDirNameErr(InvalidDirNameErr {
        name: "bad/dir".to_string(),
        trace: miru_agent::trace!(),
    })
}

fn models_err() -> ModelsErr {
    let source = chrono::DateTime::parse_from_rfc3339("invalid_datetime").unwrap_err();
    ModelsErr::DateTimeParseErr(DateTimeParseErr {
        source,
        trace: miru_agent::trace!(),
    })
}

fn storage_err() -> StorageErr {
    StorageErr::CacheErr(cache_err())
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
        assert!(matches!(err, ServiceErr::StorageErr(_)));
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
}
