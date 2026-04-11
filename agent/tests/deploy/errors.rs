// internal crates
use miru_agent::cache::errors::CacheElementNotFound;
use miru_agent::cache::CacheErr;
use miru_agent::deploy::errors::{
    BackupAccessDeniedErr, ConflictingDeploymentsErr, EmptyConfigInstancesErr, GenericErr,
    InvalidDeploymentTargetErr, PathNotAllowedErr, WriteAccessDeniedErr,
};
use miru_agent::deploy::DeployErr;
use miru_agent::filesys::errors::InvalidDirNameErr;
use miru_agent::filesys::FileSysErr;
use miru_agent::models::DplTarget;
use miru_agent::storage::StorageErr;

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

fn storage_err() -> StorageErr {
    StorageErr::CacheErr(cache_err())
}

fn empty_config_instances_err() -> EmptyConfigInstancesErr {
    EmptyConfigInstancesErr {
        deployment_id: "dpl_1".to_string(),
        trace: miru_agent::trace!(),
    }
}

fn conflicting_deployments_err() -> ConflictingDeploymentsErr {
    ConflictingDeploymentsErr {
        ids: vec!["dpl_1".to_string(), "dpl_2".to_string()],
        trace: miru_agent::trace!(),
    }
}

fn invalid_deployment_target_err() -> InvalidDeploymentTargetErr {
    InvalidDeploymentTargetErr {
        deployment_id: "dpl_1".to_string(),
        target_status: DplTarget::Archived,
        trace: miru_agent::trace!(),
    }
}

fn path_not_allowed_err() -> PathNotAllowedErr {
    PathNotAllowedErr {
        filepath: "/x".to_string(),
        reason: "test".to_string(),
        trace: miru_agent::trace!(),
    }
}

fn write_access_denied_err() -> WriteAccessDeniedErr {
    WriteAccessDeniedErr {
        cfg_inst_id: "cfg_inst_1".to_string(),
        filepath: "/locked/config.json".to_string(),
        source: Box::new(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "permission denied",
        )),
        trace: miru_agent::trace!(),
    }
}

fn backup_access_denied_err() -> BackupAccessDeniedErr {
    BackupAccessDeniedErr {
        cfg_inst_id: "cfg_inst_1".to_string(),
        filepath: "/locked/config.json".to_string(),
        backup_filepath: "/locked/miru.backup.config.json".to_string(),
        source: Box::new(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "permission denied",
        )),
        trace: miru_agent::trace!(),
    }
}

mod from_conversions {
    use super::*;

    #[test]
    fn cache_err_maps_to_deploy_cache_err() {
        let err: DeployErr = cache_err().into();
        assert!(matches!(err, DeployErr::CacheErr(_)));
    }

    #[test]
    fn filesys_err_maps_to_deploy_filesys_err() {
        let err: DeployErr = filesys_err().into();
        assert!(matches!(err, DeployErr::FileSysErr(_)));
    }

    #[test]
    fn storage_err_maps_to_deploy_storage_err() {
        let err: DeployErr = storage_err().into();
        assert!(matches!(err, DeployErr::StorageErr(_)));
    }

    #[test]
    fn empty_config_instances_err_maps_to_deploy_empty_config_instances() {
        let err: DeployErr = empty_config_instances_err().into();
        assert!(matches!(err, DeployErr::EmptyConfigInstances(_)));
    }

    #[test]
    fn invalid_deployment_target_err_maps_to_deploy_invalid_deployment_target() {
        let err: DeployErr = invalid_deployment_target_err().into();
        assert!(matches!(err, DeployErr::InvalidDeploymentTarget(_)));
    }

    #[test]
    fn conflicting_deployments_err_maps_to_deploy_conflicting_deployments() {
        let err: DeployErr = conflicting_deployments_err().into();
        assert!(matches!(err, DeployErr::ConflictingDeployments(_)));
    }

    #[test]
    fn path_not_allowed_err_maps_to_deploy_path_not_allowed() {
        let err: DeployErr = path_not_allowed_err().into();
        assert!(matches!(err, DeployErr::PathNotAllowed(_)));
    }

    #[test]
    fn generic_err_maps_to_deploy_generic_err() {
        let err: DeployErr = GenericErr {
            msg: "something went wrong".to_string(),
            trace: miru_agent::trace!(),
        }
        .into();
        assert!(matches!(err, DeployErr::GenericErr(_)));
    }

    #[test]
    fn write_access_denied_err_maps_to_deploy_write_access_denied() {
        let err: DeployErr = write_access_denied_err().into();
        assert!(matches!(err, DeployErr::WriteAccessDenied(_)));
    }

    #[test]
    fn backup_access_denied_err_maps_to_deploy_backup_access_denied() {
        let err: DeployErr = backup_access_denied_err().into();
        assert!(matches!(err, DeployErr::BackupAccessDenied(_)));
    }
}
