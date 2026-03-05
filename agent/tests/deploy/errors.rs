// internal crates
use miru_agent::cache::errors::CacheElementNotFound;
use miru_agent::cache::CacheErr;
use miru_agent::deploy::errors::{
    ConflictingDeploymentsErr, EmptyConfigInstancesErr, InvalidDeploymentTargetErr,
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
    }
}

fn conflicting_deployments_err() -> ConflictingDeploymentsErr {
    ConflictingDeploymentsErr {
        ids: vec!["dpl_1".to_string(), "dpl_2".to_string()],
    }
}

fn invalid_deployment_target_err() -> InvalidDeploymentTargetErr {
    InvalidDeploymentTargetErr {
        deployment_id: "dpl_1".to_string(),
        target_status: DplTarget::Archived,
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
}
