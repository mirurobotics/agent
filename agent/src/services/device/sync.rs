// internal crates
use crate::errors::Error;
use crate::services::errors::*;
use crate::sync::{
    errors::*,
    syncer::{State, SyncerExt},
};

/// How a requested sync ended, when it didn't fail outright.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncCode {
    Success,
    InCooldown,
    NetworkConnectionError,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SyncOutcome {
    pub code: SyncCode,
    pub message: String,
    pub state: State,
}

pub async fn sync<SyncerT: SyncerExt>(syncer: &SyncerT) -> Result<SyncOutcome, ServiceErr> {
    match syncer.sync().await {
        Ok(()) => Ok(SyncOutcome {
            code: SyncCode::Success,
            message: "successfully synced".to_string(),
            state: syncer.get_sync_state().await?,
        }),
        Err(e) => {
            let code = if matches!(e, SyncErr::InCooldownErr(_)) {
                SyncCode::InCooldown
            } else if e.is_network_conn_err() {
                SyncCode::NetworkConnectionError
            } else {
                return Err(ServiceErr::from(e));
            };

            Ok(SyncOutcome {
                code,
                message: e.to_string(),
                state: syncer.get_sync_state().await?,
            })
        }
    }
}
