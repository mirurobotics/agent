// internal crates
use crate::disk;
use crate::models;
use crate::services::{device::sync::secs_since_last_sync, errors::*};
use crate::sync::syncer::SyncerExt;

// external crates
use tracing::warn;

pub async fn get(device_stor: &disk::Device) -> Result<models::Device, ServiceErr> {
    let device = device_stor.read().await?;
    Ok((*device).clone())
}

/// Seconds since the syncer last synced in this process, or `None` before its
/// first successful sync. The syncer stops only during shutdown; if it cannot
/// be reached this also returns `None`, so reading the device still succeeds.
pub async fn get_secs_since_last_sync<SyncerT: SyncerExt>(syncer: &SyncerT) -> Option<i64> {
    match syncer.get_sync_state().await {
        Ok(sync_state) => secs_since_last_sync(&sync_state),
        Err(e) => {
            warn!("unable to read the sync state for the device response: {e}");
            None
        }
    }
}
