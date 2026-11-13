// internal crates
use crate::mocks::syncer::MockSyncer;
use miru_agent::services::device::{self as dvc_svc, SyncCode, SyncOutcome};
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
        let expected = SyncOutcome {
            code: SyncCode::InCooldown,
            message: resp.message.clone(),
            state: sync_state,
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
        let expected = SyncOutcome {
            code: SyncCode::NetworkConnectionError,
            message: resp.message.clone(),
            state: sync_state,
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

    fn expected_response(sync_state: &State) -> SyncOutcome {
        SyncOutcome {
            code: SyncCode::Success,
            message: "successfully synced".to_string(),
            state: sync_state.clone(),
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
