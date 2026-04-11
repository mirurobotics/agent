// internal crates
use crate::cache;
use crate::errors::Trace;
use crate::filesys;
use crate::models;
use crate::storage::StorageErr;

#[derive(Debug, thiserror::Error)]
#[error("deployment '{deployment_id}' has no config instances")]
pub struct EmptyConfigInstancesErr {
    pub deployment_id: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for EmptyConfigInstancesErr {}

#[derive(Debug, thiserror::Error)]
#[error("filepath '{filepath}' is not allowed: {reason}")]
pub struct PathNotAllowedErr {
    pub filepath: String,
    pub reason: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for PathNotAllowedErr {}

#[derive(Debug, thiserror::Error)]
#[error(
    "deployment '{deployment_id}' is not targeting deployed status (actual: {target_status:?})"
)]
pub struct InvalidDeploymentTargetErr {
    pub deployment_id: String,
    pub target_status: models::DplTarget,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for InvalidDeploymentTargetErr {}

#[derive(Debug, thiserror::Error)]
#[error("found {} deployments targeting deployed status (expected at most 1): [{}]", ids.len(), ids.join(", "))]
pub struct ConflictingDeploymentsErr {
    pub ids: Vec<String>,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for ConflictingDeploymentsErr {}

#[derive(Debug, thiserror::Error)]
#[error("internal server error: {msg}")]
pub struct GenericErr {
    pub msg: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for GenericErr {}

#[derive(Debug, thiserror::Error)]
#[error("duplicate filepath '{filepath}'")]
pub struct DuplicateFilepathErr {
    pub filepath: String,
    pub cfg_inst_ids: Vec<String>,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for DuplicateFilepathErr {}

#[derive(Debug, thiserror::Error)]
#[error(
    "permission denied creating backup for config instance '{cfg_inst_id}' at filepath '{filepath}' -> '{backup_filepath}': ensure that the miru user/group has read access to the existing file and write+execute access to the parent directory"
)]
pub struct BackupAccessDeniedErr {
    pub cfg_inst_id: String,
    pub filepath: String,
    pub backup_filepath: String,
    pub source: Box<std::io::Error>,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for BackupAccessDeniedErr {}

#[derive(Debug, thiserror::Error)]
#[error(
    "permission denied writing config instance '{cfg_inst_id}' to filepath '{filepath}' (or filesystem is read-only): ensure that the miru user/group has write+execute access to the parent directory"
)]
pub struct WriteAccessDeniedErr {
    pub cfg_inst_id: String,
    pub filepath: String,
    pub source: Box<std::io::Error>,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for WriteAccessDeniedErr {}

#[derive(Debug, thiserror::Error)]
pub enum DeployErr {
    #[error(transparent)]
    BackupAccessDenied(BackupAccessDeniedErr),
    #[error(transparent)]
    ConflictingDeployments(ConflictingDeploymentsErr),
    #[error(transparent)]
    DuplicateFilepath(DuplicateFilepathErr),
    #[error(transparent)]
    EmptyConfigInstances(EmptyConfigInstancesErr),
    #[error(transparent)]
    InvalidDeploymentTarget(InvalidDeploymentTargetErr),
    #[error(transparent)]
    CacheErr(cache::CacheErr),
    #[error(transparent)]
    FileSysErr(filesys::FileSysErr),
    #[error(transparent)]
    PathNotAllowed(PathNotAllowedErr),
    #[error(transparent)]
    StorageErr(StorageErr),
    #[error(transparent)]
    WriteAccessDenied(WriteAccessDeniedErr),
    #[error(transparent)]
    GenericErr(GenericErr),
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

impl From<PathNotAllowedErr> for DeployErr {
    fn from(e: PathNotAllowedErr) -> Self {
        Self::PathNotAllowed(e)
    }
}

impl From<GenericErr> for DeployErr {
    fn from(e: GenericErr) -> Self {
        Self::GenericErr(e)
    }
}

impl From<BackupAccessDeniedErr> for DeployErr {
    fn from(e: BackupAccessDeniedErr) -> Self {
        Self::BackupAccessDenied(e)
    }
}

impl From<WriteAccessDeniedErr> for DeployErr {
    fn from(e: WriteAccessDeniedErr) -> Self {
        Self::WriteAccessDenied(e)
    }
}

crate::impl_error!(DeployErr {
    BackupAccessDenied,
    ConflictingDeployments,
    DuplicateFilepath,
    EmptyConfigInstances,
    InvalidDeploymentTarget,
    CacheErr,
    FileSysErr,
    PathNotAllowed,
    StorageErr,
    WriteAccessDenied,
    GenericErr,
});
