use crate::cache;
use crate::filesys;
use crate::models;
use crate::storage::StorageErr;

#[derive(Debug, thiserror::Error)]
#[error("deployment '{deployment_id}' has no config instances")]
pub struct EmptyConfigInstancesErr {
    pub deployment_id: String,
}

impl crate::errors::Error for EmptyConfigInstancesErr {}

#[derive(Debug, thiserror::Error)]
#[error(
    "deployment '{deployment_id}' is not targeting deployed status (actual: {target_status:?})"
)]
pub struct InvalidDeploymentTargetErr {
    pub deployment_id: String,
    pub target_status: models::DplTarget,
}

impl crate::errors::Error for InvalidDeploymentTargetErr {}

#[derive(Debug, thiserror::Error)]
#[error("found {} deployments targeting deployed status (expected at most 1): [{}]", ids.len(), ids.join(", "))]
pub struct ConflictingDeploymentsErr {
    pub ids: Vec<String>,
}

impl crate::errors::Error for ConflictingDeploymentsErr {}

#[derive(Debug, thiserror::Error)]
pub enum DeployErr {
    #[error(transparent)]
    ConflictingDeployments(ConflictingDeploymentsErr),
    #[error(transparent)]
    EmptyConfigInstances(EmptyConfigInstancesErr),
    #[error(transparent)]
    InvalidDeploymentTarget(InvalidDeploymentTargetErr),
    #[error(transparent)]
    CacheErr(cache::CacheErr),
    #[error(transparent)]
    FileSysErr(filesys::FileSysErr),
    #[error(transparent)]
    StorageErr(StorageErr),
}

impl From<cache::CacheErr> for DeployErr {
    fn from(e: cache::CacheErr) -> Self {
        Self::CacheErr(e)
    }
}

impl From<filesys::FileSysErr> for DeployErr {
    fn from(e: filesys::FileSysErr) -> Self {
        Self::FileSysErr(e)
    }
}

impl From<StorageErr> for DeployErr {
    fn from(e: StorageErr) -> Self {
        Self::StorageErr(e)
    }
}

impl From<EmptyConfigInstancesErr> for DeployErr {
    fn from(e: EmptyConfigInstancesErr) -> Self {
        Self::EmptyConfigInstances(e)
    }
}

impl From<InvalidDeploymentTargetErr> for DeployErr {
    fn from(e: InvalidDeploymentTargetErr) -> Self {
        Self::InvalidDeploymentTarget(e)
    }
}

impl From<ConflictingDeploymentsErr> for DeployErr {
    fn from(e: ConflictingDeploymentsErr) -> Self {
        Self::ConflictingDeployments(e)
    }
}

crate::impl_error!(DeployErr {
    ConflictingDeployments,
    EmptyConfigInstances,
    InvalidDeploymentTarget,
    CacheErr,
    FileSysErr,
    StorageErr,
});
