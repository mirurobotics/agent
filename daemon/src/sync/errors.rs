// internal crates
use crate::authn;
use crate::cache;
use crate::deploy;
use crate::disk::DiskErr;
use crate::errors::Trace;
use crate::filesys;
use crate::http;

// external crates
use chrono::{DateTime, Utc};

#[derive(Debug, thiserror::Error)]
#[error("Sync error: {errors:?}")]
pub struct SyncErrors {
    pub errors: Vec<SyncErr>,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for SyncErrors {
    fn is_network_conn_err(&self) -> bool {
        // is only a network connection error if all errors are network connection
        // errors
        !self.errors.is_empty() && self.errors.iter().all(|e| e.is_network_conn_err())
    }
}

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
    pub is_network_conn_err: bool,
}

impl crate::errors::Error for MockErr {
    fn is_network_conn_err(&self) -> bool {
        self.is_network_conn_err
    }
}

#[derive(Debug, thiserror::Error)]
#[error("deployment '{deployment_id}' did not have config_instances expansion (backend did not expand config instances)")]
pub struct CfgInstsNotExpandedErr {
    pub deployment_id: String,
}

impl crate::errors::Error for CfgInstsNotExpandedErr {}

#[derive(Debug, thiserror::Error)]
#[error("deployment '{deployment_id}' release did not have file_rules expansion (backend did not expand release.file_rules)")]
pub struct FileRulesNotExpandedErr {
    pub deployment_id: String,
}

impl crate::errors::Error for FileRulesNotExpandedErr {}

#[derive(Debug, thiserror::Error)]
pub enum SyncErr {
    #[error(transparent)]
    AuthnErr(authn::AuthnErr),
    #[error(transparent)]
    CacheErr(cache::CacheErr),
    #[error(transparent)]
    DeployErr(Box<deploy::DeployErr>),
    #[error(transparent)]
    FileSysErr(filesys::FileSysErr),
    #[error(transparent)]
    HTTPClientErr(http::HTTPErr),
    #[error(transparent)]
    DiskErr(DiskErr),
    #[error(transparent)]
    SyncErrors(SyncErrors),
    #[error(transparent)]
    InCooldownErr(SyncerInCooldownErr),
    #[error(transparent)]
    SendActorMessageErr(SendActorMessageErr),
    #[error(transparent)]
    ReceiveActorMessageErr(ReceiveActorMessageErr),
    #[error(transparent)]
    MockErr(MockErr),
    #[error(transparent)]
    CfgInstsNotExpanded(CfgInstsNotExpandedErr),
    #[error(transparent)]
    FileRulesNotExpanded(FileRulesNotExpandedErr),
}

impl From<authn::AuthnErr> for SyncErr {
    fn from(e: authn::AuthnErr) -> Self {
        Self::AuthnErr(e)
    }
}

impl From<cache::CacheErr> for SyncErr {
    fn from(e: cache::CacheErr) -> Self {
        Self::CacheErr(e)
    }
}

impl From<deploy::DeployErr> for SyncErr {
    fn from(e: deploy::DeployErr) -> Self {
        Self::DeployErr(Box::new(e))
    }
}

impl From<filesys::FileSysErr> for SyncErr {
    fn from(e: filesys::FileSysErr) -> Self {
        Self::FileSysErr(e)
    }
}

impl From<http::HTTPErr> for SyncErr {
    fn from(e: http::HTTPErr) -> Self {
        Self::HTTPClientErr(e)
    }
}

impl From<DiskErr> for SyncErr {
    fn from(e: DiskErr) -> Self {
        Self::DiskErr(e)
    }
}

impl From<CfgInstsNotExpandedErr> for SyncErr {
    fn from(e: CfgInstsNotExpandedErr) -> Self {
        Self::CfgInstsNotExpanded(e)
    }
}

impl From<FileRulesNotExpandedErr> for SyncErr {
    fn from(e: FileRulesNotExpandedErr) -> Self {
        Self::FileRulesNotExpanded(e)
    }
}

crate::impl_error!(SyncErr {
    AuthnErr,
    CacheErr,
    DeployErr,
    FileSysErr,
    HTTPClientErr,
    DiskErr,
    SyncErrors,
    InCooldownErr,
    SendActorMessageErr,
    ReceiveActorMessageErr,
    MockErr,
    CfgInstsNotExpanded,
    FileRulesNotExpanded,
});
