use miru_agent::authn::errors::{AuthnErr, MockError as AuthnMockError};
use miru_agent::cache::errors::{CacheElementNotFound, CacheErr};
use miru_agent::deploy::errors::{DeployErr, EmptyConfigInstancesErr};
use miru_agent::filesys::errors::{FileSysErr, InvalidDirNameErr};
use miru_agent::http::errors::{HTTPErr, MockErr as HTTPMockErr};
use miru_agent::storage::StorageErr;
use miru_agent::sync::errors::SyncErr;

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

fn deploy_err() -> DeployErr {
    DeployErr::EmptyConfigInstances(EmptyConfigInstancesErr {
        deployment_id: "dpl_1".to_string(),
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

fn storage_err() -> StorageErr {
    StorageErr::CacheErr(cache_err())
}

mod from_conversions {
    use super::*;

    #[test]
    fn authn_err_maps_to_sync_authn_err() {
        let err: SyncErr = authn_err().into();
        assert!(matches!(err, SyncErr::AuthnErr(_)));
    }

    #[test]
    fn cache_err_maps_to_sync_cache_err() {
        let err: SyncErr = cache_err().into();
        assert!(matches!(err, SyncErr::CacheErr(_)));
    }

    #[test]
    fn deploy_err_maps_to_sync_deploy_err() {
        let err: SyncErr = deploy_err().into();
        assert!(matches!(err, SyncErr::DeployErr(_)));
    }

    #[test]
    fn filesys_err_maps_to_sync_filesys_err() {
        let err: SyncErr = filesys_err().into();
        assert!(matches!(err, SyncErr::FileSysErr(_)));
    }

    #[test]
    fn http_err_maps_to_sync_http_client_err() {
        let err: SyncErr = http_err().into();
        assert!(matches!(err, SyncErr::HTTPClientErr(_)));
    }

    #[test]
    fn storage_err_maps_to_sync_storage_err() {
        let err: SyncErr = storage_err().into();
        assert!(matches!(err, SyncErr::StorageErr(_)));
    }
}
