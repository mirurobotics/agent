use crate::authn::errors::AuthnErr;
use crate::cache::errors::CacheErr;
use crate::crud::errors::CrudErr;
use crate::deploy::errors::DeployErr;
use crate::errors::Trace;
use crate::filesys::errors::FileSysErr;
use crate::http::errors::HTTPErr;
use crate::storage::errors::StorageErr;

use chrono::{DateTime, Utc};

#[derive(Debug, thiserror::Error)]
#[error(
    "Missing expanded config instances: expected ids: {expected_ids:?}, actual ids: {actual_ids:?}"
)]
pub struct MissingExpandedInstancesErr {
    pub expected_ids: Vec<String>,
    pub actual_ids: Vec<String>,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for MissingExpandedInstancesErr {}

#[derive(Debug, thiserror::Error)]
pub struct SyncErrors {
    pub errors: Vec<SyncErr>,
    pub trace: Box<Trace>,
}

impl std::fmt::Display for SyncErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Sync error: {:?}", self.errors)
    }
}

impl crate::errors::Error for SyncErrors {
    fn is_network_connection_error(&self) -> bool {
        // is only a network connection error if all errors are network connection
        // errors
        for err in self.errors.iter() {
            if !err.is_network_connection_error() {
                return false;
            }
        }
        true
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Config instance content not found for config instance '{cfg_inst_id}'")]
pub struct ConfigInstanceContentNotFoundErr {
    pub cfg_inst_id: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for ConfigInstanceContentNotFoundErr {}

pub type SendActorMessageErr = crate::cache::errors::SendActorMessageErr;
pub type ReceiveActorMessageErr = crate::cache::errors::ReceiveActorMessageErr;

#[derive(Debug, thiserror::Error)]
pub struct SyncerInCooldownErr {
    pub err_streak: u32,
    pub cooldown_ends_at: DateTime<Utc>,
    pub trace: Box<Trace>,
}

impl std::fmt::Display for SyncerInCooldownErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let err_streak = self.err_streak;
        let cooldown_secs = self
            .cooldown_ends_at
            .signed_duration_since(Utc::now())
            .num_seconds();
        let cooldown_ends_at = self.cooldown_ends_at;
        write!(f, "cannot sync device because the syncer is in cooldown (err streak of {err_streak}) for {cooldown_secs} seconds (cooldown ends at: {cooldown_ends_at})",
        )
    }
}

impl crate::errors::Error for SyncerInCooldownErr {}

#[derive(Debug, thiserror::Error)]
#[error("Mock error")]
pub struct MockErr {
    pub is_network_connection_error: bool,
}

impl crate::errors::Error for MockErr {
    fn is_network_connection_error(&self) -> bool {
        self.is_network_connection_error
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SyncErr {
    #[error(transparent)]
    AuthnErr(AuthnErr),
    #[error(transparent)]
    CacheErr(CacheErr),
    #[error(transparent)]
    CrudErr(CrudErr),
    #[error(transparent)]
    DeployErr(Box<DeployErr>),
    #[error(transparent)]
    FileSysErr(FileSysErr),
    #[error(transparent)]
    HTTPClientErr(HTTPErr),
    #[error(transparent)]
    StorageErr(StorageErr),
    #[error(transparent)]
    SyncErrors(SyncErrors),
    #[error(transparent)]
    MissingExpandedInstancesErr(MissingExpandedInstancesErr),
    #[error(transparent)]
    InCooldownErr(SyncerInCooldownErr),
    #[error(transparent)]
    ConfigInstanceContentNotFound(ConfigInstanceContentNotFoundErr),
    #[error(transparent)]
    SendActorMessageErr(SendActorMessageErr),
    #[error(transparent)]
    ReceiveActorMessageErr(ReceiveActorMessageErr),
    #[error(transparent)]
    MockErr(MockErr),
}

impl From<AuthnErr> for SyncErr {
    fn from(e: AuthnErr) -> Self {
        Self::AuthnErr(e)
    }
}

impl From<CacheErr> for SyncErr {
    fn from(e: CacheErr) -> Self {
        Self::CacheErr(e)
    }
}

impl From<CrudErr> for SyncErr {
    fn from(e: CrudErr) -> Self {
        Self::CrudErr(e)
    }
}

impl From<DeployErr> for SyncErr {
    fn from(e: DeployErr) -> Self {
        Self::DeployErr(Box::new(e))
    }
}

impl From<FileSysErr> for SyncErr {
    fn from(e: FileSysErr) -> Self {
        Self::FileSysErr(e)
    }
}

impl From<HTTPErr> for SyncErr {
    fn from(e: HTTPErr) -> Self {
        Self::HTTPClientErr(e)
    }
}

impl From<StorageErr> for SyncErr {
    fn from(e: StorageErr) -> Self {
        Self::StorageErr(e)
    }
}

crate::impl_error!(SyncErr {
    AuthnErr,
    CacheErr,
    CrudErr,
    DeployErr,
    FileSysErr,
    HTTPClientErr,
    StorageErr,
    SyncErrors,
    MissingExpandedInstancesErr,
    InCooldownErr,
    ConfigInstanceContentNotFound,
    SendActorMessageErr,
    ReceiveActorMessageErr,
    MockErr,
});
