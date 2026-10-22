// standard crates
use std::time::Duration;

// internal crates
use crate::errors::Error;
use crate::services::errors::*;
use crate::sync::{
    errors::*,
    syncer::{State, SyncerExt},
};
use device_api::models::{SyncDeviceResponse, SyncDeviceResult};

pub async fn sync<SyncerT: SyncerExt>(syncer: &SyncerT) -> Result<SyncDeviceResponse, ServiceErr> {
    let (code, message) = match syncer.sync().await {
        Ok(()) => (
            SyncDeviceResult::SYNC_DEVICE_RESULT_SUCCESS,
            "successfully synced".to_string(),
        ),
        Err(e) => {
            let code = if matches!(e, SyncErr::InCooldownErr(_)) {
                SyncDeviceResult::SYNC_DEVICE_RESULT_IN_COOLDOWN
            } else if e.is_network_conn_err() {
                SyncDeviceResult::SYNC_DEVICE_RESULT_NETWORK_CONNECTION_ERROR
            } else {
                return Err(ServiceErr::from(e));
            };
            (code, e.to_string())
        }
    };

    let sync_state = syncer.get_sync_state().await?;
    Ok(SyncDeviceResponse {
        code,
        message,
        last_synced_at: sync_state.last_synced_at.to_rfc3339(),
        last_attempted_sync_at: sync_state.last_attempted_sync_at.to_rfc3339(),
        in_cooldown: sync_state.is_in_cooldown(),
        cooldown_ends_at: sync_state.cooldown_ends_at.to_rfc3339(),
        remaining_cooldown_secs: remaining_cooldown_secs(&sync_state),
        secs_since_last_sync: secs_since_last_sync(&sync_state),
    })
}

/// Whole seconds left in the syncer's cooldown, rounded up so the value is 0
/// only once the cooldown has ended.
pub fn remaining_cooldown_secs(sync_state: &State) -> i64 {
    to_secs(ceil_secs(sync_state.remaining_cooldown()))
}

/// Whole seconds since the syncer last synced in this process, rounded down,
/// or `None` before its first successful sync.
pub fn secs_since_last_sync(sync_state: &State) -> Option<i64> {
    sync_state
        .since_last_sync()
        .map(|elapsed| to_secs(elapsed.as_secs()))
}

fn ceil_secs(duration: Duration) -> u64 {
    let secs = duration.as_secs();
    if duration.subsec_nanos() > 0 {
        secs.saturating_add(1)
    } else {
        secs
    }
}

fn to_secs(secs: u64) -> i64 {
    i64::try_from(secs).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ceil_secs_rounds_partial_seconds_up() {
        assert_eq!(ceil_secs(Duration::ZERO), 0);
        assert_eq!(ceil_secs(Duration::from_millis(1)), 1);
        assert_eq!(ceil_secs(Duration::from_secs(5)), 5);
        assert_eq!(ceil_secs(Duration::from_millis(5_001)), 6);
        assert_eq!(ceil_secs(Duration::new(u64::MAX, 1)), u64::MAX);
    }

    #[test]
    fn to_secs_saturates_at_i64_max() {
        assert_eq!(to_secs(42), 42);
        assert_eq!(to_secs(u64::MAX), i64::MAX);
    }
}
