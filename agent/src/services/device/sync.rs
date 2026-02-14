// internal crates
use crate::sync::{errors::*, syncer::SyncerExt};
use crate::{errors::Error, services::errors::*};
use openapi_server::models::{SyncDeviceResponse, SyncDeviceResult};

pub async fn sync_device<SyncerT: SyncerExt>(
    syncer: &SyncerT,
) -> Result<SyncDeviceResponse, ServiceErr> {
    match syncer.sync().await {
        Ok(()) => {
            let sync_state = syncer.get_sync_state().await?;
            Ok(SyncDeviceResponse {
                code: SyncDeviceResult::SYNC_DEVICE_RESULT_SUCCESS,
                message: "successfully synced".to_string(),
                last_synced_at: sync_state.last_synced_at.to_rfc3339(),
                last_attempted_sync_at: sync_state.last_attempted_sync_at.to_rfc3339(),
                in_cooldown: sync_state.is_in_cooldown(),
                cooldown_ends_at: sync_state.cooldown_ends_at.to_rfc3339(),
            })
        }
        Err(e) => {
            let code = if matches!(e, SyncErr::InCooldownErr(_)) {
                SyncDeviceResult::SYNC_DEVICE_RESULT_IN_COOLDOWN
            } else if e.is_network_connection_error() {
                SyncDeviceResult::SYNC_DEVICE_RESULT_NETWORK_CONNECTION_ERROR
            } else {
                return Err(ServiceErr::from(e));
            };

            let sync_state = syncer.get_sync_state().await?;

            Ok(SyncDeviceResponse {
                code,
                message: e.to_string(),
                last_synced_at: sync_state.last_synced_at.to_rfc3339(),
                last_attempted_sync_at: sync_state.last_attempted_sync_at.to_rfc3339(),
                in_cooldown: sync_state.is_in_cooldown(),
                cooldown_ends_at: sync_state.cooldown_ends_at.to_rfc3339(),
            })
        }
    }
}
