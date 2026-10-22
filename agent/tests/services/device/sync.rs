// standard crates
use std::time::{Duration, Instant};

// internal crates
use crate::mocks::syncer::MockSyncer;
use device_api::models::{SyncDeviceResponse, SyncDeviceResult};
use miru_agent::services::device as dvc_svc;
use miru_agent::sync::errors::{MockErr, SyncerInCooldownErr};
use miru_agent::sync::syncer::State;
use miru_agent::sync::SyncErr;
use miru_agent::trace;

// external crates
use chrono::{DateTime, TimeDelta, Utc};

/// Asserts `resp` matches `expected` in every field but `cooldown_ends_at`,
/// which is the system clock at response time plus the remaining cooldown, so
/// it is checked to fall within `[before + remaining - 1s, after + remaining]`.
fn assert_response(
    resp: &SyncDeviceResponse,
    expected: SyncDeviceResponse,
    before: DateTime<Utc>,
    remaining: TimeDelta,
) {
    let after = Utc::now();
    let ends_at = DateTime::parse_from_rfc3339(&resp.cooldown_ends_at)
        .unwrap()
        .with_timezone(&Utc);
    assert!(
        ends_at >= before + remaining - TimeDelta::seconds(1) && ends_at <= after + remaining,
        "cooldown_ends_at {ends_at} not within [{before}, {after}] + {remaining}"
    );
    let expected = SyncDeviceResponse {
        cooldown_ends_at: resp.cooldown_ends_at.clone(),
        ..expected
    };
    assert_eq!(*resp, expected);
}

pub mod errors {
    use super::*;

    #[tokio::test]
    async fn device_in_cooldown() {
        let syncer = MockSyncer::default();
        let day = Duration::from_secs(24 * 60 * 60);
        let sync_state = State {
            cooldown_ends: Some(Instant::now() + day),
            ..State::default()
        };
        syncer.set_state(sync_state.clone());

        let err_state = sync_state.clone();
        syncer.set_sync(move || {
            Err(SyncErr::InCooldownErr(SyncerInCooldownErr {
                remaining_cooldown: err_state.remaining_cooldown(),
                cooldown_ends_at: err_state.cooldown_ends_at(),
                err_streak: 0,
                trace: trace!(),
            }))
        });

        let before = Utc::now();
        let resp = dvc_svc::sync(&syncer).await.unwrap();
        let expected = SyncDeviceResponse {
            code: SyncDeviceResult::SYNC_DEVICE_RESULT_IN_COOLDOWN,
            message: resp.message.clone(),
            last_synced_at: sync_state.last_synced_at.to_rfc3339(),
            last_attempted_sync_at: sync_state.last_attempted_sync_at.to_rfc3339(),
            in_cooldown: true,
            cooldown_ends_at: String::new(),
        };

        assert_response(&resp, expected, before, TimeDelta::days(1));
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

        let before = Utc::now();
        let resp = dvc_svc::sync(&syncer).await.unwrap();
        let expected = SyncDeviceResponse {
            code: SyncDeviceResult::SYNC_DEVICE_RESULT_NETWORK_CONNECTION_ERROR,
            message: resp.message.clone(),
            last_synced_at: sync_state.last_synced_at.to_rfc3339(),
            last_attempted_sync_at: sync_state.last_attempted_sync_at.to_rfc3339(),
            in_cooldown: sync_state.is_in_cooldown(),
            cooldown_ends_at: String::new(),
        };

        assert_response(&resp, expected, before, TimeDelta::zero());
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
            cooldown_ends_at: String::new(),
        }
    }

    #[tokio::test]
    async fn default_state() {
        let syncer = MockSyncer::default();
        let sync_state = State::default();
        syncer.set_state(sync_state.clone());

        let before = Utc::now();
        let resp = dvc_svc::sync(&syncer).await.unwrap();

        assert_response(
            &resp,
            expected_response(&sync_state),
            before,
            TimeDelta::zero(),
        );
    }

    #[tokio::test]
    async fn non_default_state() {
        let syncer = MockSyncer::default();
        let now = Utc::now();
        let sync_state = State {
            last_synced_at: now - TimeDelta::minutes(5),
            last_attempted_sync_at: now - TimeDelta::minutes(1),
            cooldown_ends: Some(Instant::now() + Duration::from_secs(60 * 60)),
            ..State::default()
        };
        syncer.set_state(sync_state.clone());

        let before = Utc::now();
        let resp = dvc_svc::sync(&syncer).await.unwrap();

        assert!(resp.in_cooldown);
        assert_response(
            &resp,
            expected_response(&sync_state),
            before,
            TimeDelta::hours(1),
        );
    }
}
