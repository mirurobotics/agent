use miru_agent::authn::errors::MockError as AuthnMockError;
use miru_agent::authn::AuthnErr;
use miru_agent::cache::errors::CacheElementNotFound;
use miru_agent::cache::CacheErr;
use miru_agent::deploy::errors::EmptyConfigInstancesErr;
use miru_agent::deploy::DeployErr;
use miru_agent::errors::Error;
use miru_agent::filesys::errors::InvalidDirNameErr;
use miru_agent::filesys::FileSysErr;
use miru_agent::http::errors::{HTTPErr, MockErr as HTTPMockErr};
use miru_agent::storage::StorageErr;
use miru_agent::sync::errors::{CfgInstsNotExpandedErr, SyncErrors, SyncerInCooldownErr};
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

    #[test]
    fn cfg_insts_not_expanded_err_maps() {
        let err: SyncErr = CfgInstsNotExpandedErr {
            deployment_id: "dpl_1".to_string(),
        }
        .into();
        assert!(matches!(err, SyncErr::CfgInstsNotExpanded(_)));
    }
}

mod is_network_conn_err {
    use super::*;

    fn network_sync_err() -> SyncErr {
        SyncErr::HTTPClientErr(HTTPErr::MockErr(HTTPMockErr {
            is_network_conn_err: true,
        }))
    }

    fn non_network_sync_err() -> SyncErr {
        SyncErr::HTTPClientErr(HTTPErr::MockErr(HTTPMockErr {
            is_network_conn_err: false,
        }))
    }

    #[test]
    fn all_network_errors_returns_true() {
        let errs = SyncErrors {
            errors: vec![network_sync_err(), network_sync_err()],
            trace: miru_agent::trace!(),
        };
        assert!(errs.is_network_conn_err());
    }

    #[test]
    fn any_non_network_error_returns_false() {
        let errs = SyncErrors {
            errors: vec![network_sync_err(), non_network_sync_err()],
            trace: miru_agent::trace!(),
        };
        assert!(!errs.is_network_conn_err());
    }

    #[test]
    fn empty_errors_returns_false() {
        let errs = SyncErrors {
            errors: vec![],
            trace: miru_agent::trace!(),
        };
        assert!(!errs.is_network_conn_err());
    }
}

mod display {
    use super::*;

    #[test]
    fn syncer_in_cooldown_err_display() {
        let err = SyncerInCooldownErr {
            err_streak: 3,
            cooldown_ends_at: chrono::Utc::now() + chrono::TimeDelta::seconds(60),
            trace: miru_agent::trace!(),
        };
        let msg = format!("{err}");
        assert!(
            msg.contains("syncer is in cooldown"),
            "display should mention cooldown, got: {msg}"
        );
        assert!(
            msg.contains("err streak of 3"),
            "display should mention err streak, got: {msg}"
        );
    }
}
