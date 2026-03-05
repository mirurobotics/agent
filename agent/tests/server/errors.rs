// internal crates
use miru_agent::authn::errors::MockError as AuthnMockError;
use miru_agent::authn::AuthnErr;
use miru_agent::cache::errors::CacheElementNotFound;
use miru_agent::cache::CacheErr;
use miru_agent::crypt::errors::InvalidJWTErr;
use miru_agent::crypt::CryptErr;
use miru_agent::filesys::errors::InvalidDirNameErr;
use miru_agent::filesys::FileSysErr;
use miru_agent::http::errors::MockErr as HTTPMockErr;
use miru_agent::http::HTTPErr;
use miru_agent::server::ServerErr;
use miru_agent::services::ServiceErr;
use miru_agent::storage::StorageErr;
use miru_agent::sync::errors::MockErr as SyncMockErr;
use miru_agent::sync::SyncErr;

fn authn_err() -> AuthnErr {
    AuthnErr::MockError(AuthnMockError {
        is_network_conn_err: false,
        trace: miru_agent::trace!(),
    })
}

fn cache_err() -> CacheErr {
    CacheErr::CacheElementNotFound(CacheElementNotFound {
        msg: "cache miss".to_string(),
        trace: miru_agent::trace!(),
    })
}

fn crypt_err() -> CryptErr {
    CryptErr::InvalidJWTErr(InvalidJWTErr {
        msg: "invalid jwt".to_string(),
        trace: miru_agent::trace!(),
    })
}

fn filesys_err() -> FileSysErr {
    FileSysErr::InvalidDirNameErr(InvalidDirNameErr {
        name: "bad/dir".to_string(),
        trace: miru_agent::trace!(),
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

fn storage_err() -> StorageErr {
    StorageErr::CacheErr(cache_err())
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
        assert!(matches!(err, ServerErr::StorageErr(_)));
    }

    #[test]
    fn sync_err_maps_to_server_sync_err() {
        let err: ServerErr = sync_err().into();
        assert!(matches!(err, ServerErr::SyncErr(_)));
    }
}
