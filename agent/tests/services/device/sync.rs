// internal crates
use crate::sync::mock::MockSyncer;
use device_api::models::{SyncDeviceResponse, SyncDeviceResult};
use miru_agent::services::device as dvc_svc;
use miru_agent::sync::errors::{MockErr, SyncerInCooldownErr};
use miru_agent::sync::syncer::State;
use miru_agent::sync::SyncErr;
use miru_agent::trace;

// external crates
use chrono::{Duration, Utc};

pub mod errors {
    use super::*;

    #[tokio::test]
    async fn device_in_cooldown() {
        let syncer = MockSyncer::default();
        let sync_state = State {
            cooldown_ends_at: Utc::now() + Duration::days(1),
            ..State::default()
        };
        syncer.set_state(sync_state.clone());

        syncer.set_sync(move || {
            Err(SyncErr::InCooldownErr(SyncerInCooldownErr {
                cooldown_ends_at: sync_state.cooldown_ends_at,
                err_streak: 0,
                trace: trace!(),
            }))
        });

        let resp = dvc_svc::sync(&syncer).await.unwrap();
        let expected = SyncDeviceResponse {
            code: SyncDeviceResult::SYNC_DEVICE_RESULT_IN_COOLDOWN,
            message: resp.message.clone(),
            last_synced_at: sync_state.last_synced_at.to_rfc3339(),
            last_attempted_sync_at: sync_state.last_attempted_sync_at.to_rfc3339(),
            in_cooldown: true,
            cooldown_ends_at: sync_state.cooldown_ends_at.to_rfc3339(),
        };

        assert_eq!(resp, expected);
    }

    #[tokio::test]
    async fn network_connection_error() {
        let syncer = MockSyncer::default();
        let sync_state = State::default();
        syncer.set_state(sync_state.clone());

        syncer.set_sync(move || {
            Err(SyncErr::MockErr(MockErr {
                is_network_conn_err: true,
            }))
        });

        let resp = dvc_svc::sync(&syncer).await.unwrap();
        let expected = SyncDeviceResponse {
            code: SyncDeviceResult::SYNC_DEVICE_RESULT_NETWORK_CONNECTION_ERROR,
            message: resp.message.clone(),
            last_synced_at: sync_state.last_synced_at.to_rfc3339(),
            last_attempted_sync_at: sync_state.last_attempted_sync_at.to_rfc3339(),
            in_cooldown: sync_state.is_in_cooldown(),
            cooldown_ends_at: sync_state.cooldown_ends_at.to_rfc3339(),
        };

        assert_eq!(resp, expected);
    }

    #[tokio::test]
    async fn internal_server_error() {
        let syncer = MockSyncer::default();
        let sync_state = State::default();
        syncer.set_state(sync_state.clone());

        syncer.set_sync(move || {
            Err(SyncErr::MockErr(MockErr {
                is_network_conn_err: false,
            }))
        });

        dvc_svc::sync(&syncer).await.unwrap_err();
    }
}

pub mod success {
    use super::*;

    fn expected_response(sync_state: &State) -> SyncDeviceResponse {
        SyncDeviceResponse {
            code: SyncDeviceResult::SYNC_DEVICE_RESULT_SUCCESS,
            message: "successfully synced".to_string(),
            last_synced_at: sync_state.last_synced_at.to_rfc3339(),
            last_attempted_sync_at: sync_state.last_attempted_sync_at.to_rfc3339(),
            in_cooldown: sync_state.is_in_cooldown(),
            cooldown_ends_at: sync_state.cooldown_ends_at.to_rfc3339(),
        }
    }

    #[tokio::test]
    async fn default_state() {
        let syncer = MockSyncer::default();
        let sync_state = State::default();
        syncer.set_state(sync_state.clone());

        let resp = dvc_svc::sync(&syncer).await.unwrap();

        assert_eq!(resp, expected_response(&sync_state));
    }

    #[tokio::test]
    async fn non_default_state() {
        let syncer = MockSyncer::default();
        let now = Utc::now();
        let sync_state = State {
            last_synced_at: now - Duration::minutes(5),
            last_attempted_sync_at: now - Duration::minutes(1),
            cooldown_ends_at: now - Duration::hours(1),
            ..State::default()
        };
        syncer.set_state(sync_state.clone());

        let resp = dvc_svc::sync(&syncer).await.unwrap();

        assert_eq!(resp, expected_response(&sync_state));
    }
}
