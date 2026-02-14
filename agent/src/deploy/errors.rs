use crate::cache::errors::CacheErr;
use crate::crud::errors::CrudErr;
use crate::deploy::fsm;
use crate::errors::Trace;
use crate::filesys::errors::FileSysErr;
use crate::models::deployment::Deployment;
use crate::storage::errors::StorageErr;

#[derive(Debug, thiserror::Error)]
pub struct DeploymentNotDeployableErr {
    pub deployment: Deployment,
    pub next_action: fsm::NextAction,
    pub trace: Box<Trace>,
}

impl std::fmt::Display for DeploymentNotDeployableErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "cannot deploy deployment '{:?}' since it's next action is: {:?}",
            self.deployment.id, self.next_action
        )
    }
}

impl crate::errors::Error for DeploymentNotDeployableErr {}

#[derive(Debug, thiserror::Error)]
pub struct DeploymentNotRemoveableErr {
    pub deployment: Deployment,
    pub next_action: fsm::NextAction,
    pub trace: Box<Trace>,
}

impl std::fmt::Display for DeploymentNotRemoveableErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "cannot remove deployment '{:?}' since it's next action is: {:?}",
            self.deployment.id, self.next_action
        )
    }
}

impl crate::errors::Error for DeploymentNotRemoveableErr {}

#[derive(Debug, thiserror::Error)]
pub struct DeploymentNotArchiveableErr {
    pub deployment: Deployment,
    pub next_action: fsm::NextAction,
    pub trace: Box<Trace>,
}

impl std::fmt::Display for DeploymentNotArchiveableErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "cannot archive deployment '{:?}' since it's next action is: {:?}",
            self.deployment.id, self.next_action
        )
    }
}

impl crate::errors::Error for DeploymentNotArchiveableErr {}

#[derive(Debug, thiserror::Error)]
pub struct ConflictingDeploymentsErr {
    pub deployments: Vec<Deployment>,
    pub trace: Box<Trace>,
}

impl std::fmt::Display for ConflictingDeploymentsErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the following deployments both desire to be deployed: {:?}",
            self.deployments.iter().map(|d| &d.id).collect::<Vec<_>>()
        )
    }
}

impl crate::errors::Error for ConflictingDeploymentsErr {}

#[derive(Debug, thiserror::Error)]
pub enum DeployErr {
    #[error(transparent)]
    ConflictingDeploymentsErr(ConflictingDeploymentsErr),
    #[error(transparent)]
    DeploymentNotDeployableErr(DeploymentNotDeployableErr),
    #[error(transparent)]
    DeploymentNotRemoveableErr(DeploymentNotRemoveableErr),
    #[error(transparent)]
    DeploymentNotArchiveableErr(DeploymentNotArchiveableErr),
    #[error(transparent)]
    CacheErr(CacheErr),
    #[error(transparent)]
    CrudErr(CrudErr),
    #[error(transparent)]
    FileSysErr(FileSysErr),
    #[error(transparent)]
    StorageErr(StorageErr),
}

impl From<CacheErr> for DeployErr {
    fn from(e: CacheErr) -> Self {
        Self::CacheErr(e)
    }
}

impl From<CrudErr> for DeployErr {
    fn from(e: CrudErr) -> Self {
        Self::CrudErr(e)
    }
}

impl From<FileSysErr> for DeployErr {
    fn from(e: FileSysErr) -> Self {
        Self::FileSysErr(e)
    }
}

impl From<StorageErr> for DeployErr {
    fn from(e: StorageErr) -> Self {
        Self::StorageErr(e)
    }
}

crate::impl_error!(DeployErr {
    ConflictingDeploymentsErr,
    DeploymentNotDeployableErr,
    DeploymentNotRemoveableErr,
    DeploymentNotArchiveableErr,
    CacheErr,
    CrudErr,
    FileSysErr,
    StorageErr,
});
