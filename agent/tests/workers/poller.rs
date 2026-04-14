// standard crates
use std::sync::Arc;

// internal crates
use crate::mocks::{error::SleepController, syncer::MockSyncer};
use miru_agent::filesys;
use miru_agent::models::Device;
use miru_agent::storage::{self, Layout};
use miru_agent::sync::errors::MockErr as SyncMockErr;
use miru_agent::sync::syncer::{CooldownEnd, State, SyncEvent, SyncFailure};
use miru_agent::sync::SyncErr;
use miru_agent::workers::poller;

// external crates
use chrono::{TimeDelta, Utc};

pub mod run {
    use super::*;

    #[tokio::test]
    async fn syncer_not_in_cooldown() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);

        let (device_file, _) =
            storage::Device::spawn_with_default(64, layout.device(), Device::default())
                .await
                .unwrap();

        let options = poller::Options::default();
        let syncer = Arc::new(MockSyncer::default());
        let sleep_ctrl = Arc::new(SleepController::new());

        let options_for_spawn = options.clone();
        let syncer_for_spawn = syncer.clone();
        let sleep_ctrl_for_spawn = sleep_ctrl.clone();
        let shutdown_signal = Box::pin(async move {
            std::future::pending::<()>().await;
        });
        let _handle = tokio::spawn(async move {
            poller::run(
                &options_for_spawn,
                syncer_for_spawn.as_ref(),
                &device_file,
                sleep_ctrl_for_spawn.sleep_fn(),
                shutdown_signal,
            )
            .await;
        });

        let secs_since_last_sync = 30;
        let state = State {
            last_attempted_sync_at: Utc::now() - TimeDelta::seconds(secs_since_last_sync),
            last_synced_at: Utc::now(),
            cooldown_ends_at: Utc::now(),
            err_streak: 0,
        };
        syncer.set_state(state);

        // these sleeps should wait for the polling interval since it exceeds the syncer
        let expected_sleep_secs = options.poll_interval_secs - secs_since_last_sync;
        // cooldown
        for i in 0..10 {
            sleep_ctrl.await_sleep().await;
            let last_sleep = sleep_ctrl.get_last_attempted_sleep().unwrap();
            assert!(last_sleep.as_secs() <= expected_sleep_secs as u64);
            assert!(last_sleep.as_secs() >= expected_sleep_secs as u64 - 1);
            assert_eq!(syncer.num_sync_calls(), i + 1);
            sleep_ctrl.release().await;
        }

        // these sleeps should still wait for the polling interval starting from the
        // last sync attempt since errors are logged & ignored
        syncer.set_sync(|| {
            Err(SyncErr::MockErr(SyncMockErr {
                is_network_conn_err: true,
            }))
        });
        for i in 0..10 {
            sleep_ctrl.await_sleep().await;
            let last_sleep = sleep_ctrl.get_last_attempted_sleep().unwrap();
            assert!(last_sleep.as_secs() <= expected_sleep_secs as u64);
            assert!(last_sleep.as_secs() >= expected_sleep_secs as u64 - 1);
            assert_eq!(syncer.num_sync_calls(), i + 11);
            sleep_ctrl.release().await;
        }

        syncer.set_sync(|| {
            Err(SyncErr::MockErr(SyncMockErr {
                is_network_conn_err: false,
            }))
        });
        for i in 0..10 {
            sleep_ctrl.await_sleep().await;
            let last_sleep = sleep_ctrl.get_last_attempted_sleep().unwrap();
            assert!(last_sleep.as_secs() <= expected_sleep_secs as u64);
            assert!(last_sleep.as_secs() >= expected_sleep_secs as u64 - 1);
            assert_eq!(syncer.num_sync_calls(), i + 21);
            sleep_ctrl.release().await;
        }
    }

    #[tokio::test]
    async fn syncer_in_cooldown() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);

        let (device_file, _) =
            storage::Device::spawn_with_default(64, layout.device(), Device::default())
                .await
                .unwrap();

        let options = poller::Options {
            poll_interval_secs: 30,
        };
        let options_for_spawn = options.clone();
        let syncer = Arc::new(MockSyncer::default());
        let sleep_ctrl = Arc::new(SleepController::new());

        let syncer_for_spawn = syncer.clone();
        let sleep_ctrl_for_spawn = sleep_ctrl.clone();
        let shutdown_signal = Box::pin(async move {
            std::future::pending::<()>().await;
        });
        let _handle = tokio::spawn(async move {
            poller::run(
                &options_for_spawn,
                syncer_for_spawn.as_ref(),
                &device_file,
                sleep_ctrl_for_spawn.sleep_fn(),
                shutdown_signal,
            )
            .await;
        });

        let secs_until_cooldown_ends = 120;
        let state = State {
            last_attempted_sync_at: Utc::now(),
            last_synced_at: Utc::now(),
            cooldown_ends_at: Utc::now() + TimeDelta::seconds(secs_until_cooldown_ends),
            err_streak: 0,
        };
        syncer.set_state(state);

        // these sleeps should wait for the syncer cooldown to end since it's greater
        // than the polling interval
        for _ in 0..10 {
            sleep_ctrl.await_sleep().await;
            let last_sleep = sleep_ctrl.get_last_attempted_sleep().unwrap();
            assert!(last_sleep.as_secs() <= secs_until_cooldown_ends as u64);
            assert!(last_sleep.as_secs() >= secs_until_cooldown_ends as u64 - 1);
            assert_eq!(syncer.num_sync_calls(), 0); // syncer in cooldown
            sleep_ctrl.release().await;
        }

        // these sleeps should still wait for the syncer cooldown to end since errors
        // are logged & ignored
        syncer.set_sync(|| {
            Err(SyncErr::MockErr(SyncMockErr {
                is_network_conn_err: true,
            }))
        });
        for _ in 0..10 {
            sleep_ctrl.await_sleep().await;
            let last_sleep = sleep_ctrl.get_last_attempted_sleep().unwrap();
            assert!(last_sleep.as_secs() <= secs_until_cooldown_ends as u64);
            assert!(last_sleep.as_secs() >= secs_until_cooldown_ends as u64 - 1);
            assert_eq!(syncer.num_sync_calls(), 0); // syncer in cooldown
            sleep_ctrl.release().await;
        }

        syncer.set_sync(|| {
            Err(SyncErr::MockErr(SyncMockErr {
                is_network_conn_err: false,
            }))
        });
        for _ in 0..10 {
            sleep_ctrl.await_sleep().await;
            let last_sleep = sleep_ctrl.get_last_completed_sleep().unwrap();
            assert!(last_sleep.as_secs() <= secs_until_cooldown_ends as u64);
            assert!(last_sleep.as_secs() >= secs_until_cooldown_ends as u64 - 1);
            assert_eq!(syncer.num_sync_calls(), 0); // syncer in cooldown
            sleep_ctrl.release().await;
        }
    }

    #[tokio::test]
    async fn sync_success_event_updates_last_synced_at() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);

        let (device_file, _) =
            storage::Device::spawn_with_default(64, layout.device(), Device::default())
                .await
                .unwrap();
        let device_file = Arc::new(device_file);

        let options = poller::Options::default();
        let syncer = Arc::new(MockSyncer::default());
        let sleep_ctrl = Arc::new(SleepController::new());

        let options_for_spawn = options.clone();
        let syncer_for_spawn = syncer.clone();
        let sleep_ctrl_for_spawn = sleep_ctrl.clone();
        let device_file_for_spawn = device_file.clone();
        let shutdown_signal = Box::pin(async move {
            std::future::pending::<()>().await;
        });
        let _handle = tokio::spawn(async move {
            poller::run(
                &options_for_spawn,
                syncer_for_spawn.as_ref(),
                &device_file_for_spawn,
                sleep_ctrl_for_spawn.sleep_fn(),
                shutdown_signal,
            )
            .await;
        });

        let secs_since_last_sync = 30;
        let state = State {
            last_attempted_sync_at: Utc::now() - TimeDelta::seconds(secs_since_last_sync),
            last_synced_at: Utc::now(),
            cooldown_ends_at: Utc::now(),
            err_streak: 0,
        };
        syncer.set_state(state);

        let syncer_tx = syncer.get_transmitter();

        // wait for the poller to enter sleep after initial sync
        sleep_ctrl.await_sleep().await;

        // verify last_synced_at is at UNIX_EPOCH before the event
        let device_before = device_file.read().await.unwrap();
        assert_eq!(
            device_before.last_synced_at,
            chrono::DateTime::<Utc>::UNIX_EPOCH
        );

        // send SyncSuccess event while the poller is sleeping
        let before_event = Utc::now();
        syncer_tx.send(SyncEvent::SyncSuccess).unwrap();
        sleep_ctrl.release().await;

        // wait for the poller to process the event and re-enter sleep
        sleep_ctrl.await_sleep().await;

        // verify last_synced_at was updated
        let device_after = device_file.read().await.unwrap();
        assert!(
            device_after.last_synced_at >= before_event,
            "last_synced_at should be updated after SyncSuccess event"
        );
        assert!(device_after.last_synced_at <= Utc::now());
    }

    #[tokio::test]
    async fn cooldown_end_from_deployment_wait_triggers_sync() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);

        let (device_file, _) =
            storage::Device::spawn_with_default(64, layout.device(), Device::default())
                .await
                .unwrap();

        let options = poller::Options::default();
        let syncer = Arc::new(MockSyncer::default());
        let sleep_ctrl = Arc::new(SleepController::new());

        let options_for_spawn = options.clone();
        let syncer_for_spawn = syncer.clone();
        let sleep_ctrl_for_spawn = sleep_ctrl.clone();
        let shutdown_signal = Box::pin(async move {
            std::future::pending::<()>().await;
        });
        let _handle = tokio::spawn(async move {
            poller::run(
                &options_for_spawn,
                syncer_for_spawn.as_ref(),
                &device_file,
                sleep_ctrl_for_spawn.sleep_fn(),
                shutdown_signal,
            )
            .await;
        });

        let state = State {
            last_attempted_sync_at: Utc::now(),
            last_synced_at: Utc::now(),
            cooldown_ends_at: Utc::now() - TimeDelta::seconds(10),
            err_streak: 0,
        };
        syncer.set_state(state);

        let syncer_tx = syncer.get_transmitter();

        // wait for the poller to enter sleep after initial sync
        sleep_ctrl.await_sleep().await;
        let sync_calls_before = syncer.num_sync_calls();

        // send DeploymentWait cooldown end event
        syncer_tx
            .send(SyncEvent::CooldownEnd(CooldownEnd::DeploymentWait))
            .unwrap();
        sleep_ctrl.release().await;

        // wait for the poller to process the event and re-enter sleep
        sleep_ctrl.await_sleep().await;

        // DeploymentWait should trigger a sync
        assert!(
            syncer.num_sync_calls() > sync_calls_before,
            "CooldownEnd::DeploymentWait should trigger a sync"
        );
    }

    #[tokio::test]
    async fn ignored_syncer_events() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);

        let (device_file, _) =
            storage::Device::spawn_with_default(64, layout.device(), Device::default())
                .await
                .unwrap();

        let options = poller::Options::default();
        let syncer = Arc::new(MockSyncer::default());
        let sleep_ctrl = Arc::new(SleepController::new());

        let options_for_spawn = options.clone();
        let syncer_for_spawn = syncer.clone();
        let sleep_ctrl_for_spawn = sleep_ctrl.clone();
        let shutdown_signal = Box::pin(async move {
            std::future::pending::<()>().await;
        });
        let _handle = tokio::spawn(async move {
            poller::run(
                &options_for_spawn,
                syncer_for_spawn.as_ref(),
                &device_file,
                sleep_ctrl_for_spawn.sleep_fn(),
                shutdown_signal,
            )
            .await;
        });

        let secs_since_last_sync = 30;
        let state = State {
            last_attempted_sync_at: Utc::now() - TimeDelta::seconds(secs_since_last_sync),
            last_synced_at: Utc::now(),
            cooldown_ends_at: Utc::now(),
            err_streak: 0,
        };
        syncer.set_state(state);

        let syncer_tx = syncer.get_transmitter();

        let expected_sleep_secs = options.poll_interval_secs - secs_since_last_sync;
        let expected_num_sync_calls = 1; // only the first sync occurs
        for event in [
            SyncEvent::SyncSuccess,
            SyncEvent::SyncFailed(SyncFailure {
                is_network_conn_err: true,
            }),
            SyncEvent::CooldownEnd(CooldownEnd::SyncSuccess),
        ] {
            for _ in 0..10 {
                sleep_ctrl.await_sleep().await;
                let last_sleep = sleep_ctrl.get_last_attempted_sleep().unwrap();
                assert!(last_sleep.as_secs() <= expected_sleep_secs as u64);
                assert!(last_sleep.as_secs() >= expected_sleep_secs as u64 - 1);
                assert_eq!(syncer.num_sync_calls(), expected_num_sync_calls);
                syncer_tx.send(event.clone()).unwrap();
            }
        }
    }

    #[tokio::test]
    async fn syncer_cooldown_end_from_sync_failure_event() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);

        let (device_file, _) =
            storage::Device::spawn_with_default(64, layout.device(), Device::default())
                .await
                .unwrap();

        let options = poller::Options::default();
        let syncer = Arc::new(MockSyncer::default());
        let sleep_ctrl = Arc::new(SleepController::new());

        let options_for_spawn = options.clone();
        let syncer_for_spawn = syncer.clone();
        let sleep_ctrl_for_spawn = sleep_ctrl.clone();
        let shutdown_signal = Box::pin(async move {
            std::future::pending::<()>().await;
        });
        let _handle = tokio::spawn(async move {
            poller::run(
                &options_for_spawn,
                syncer_for_spawn.as_ref(),
                &device_file,
                sleep_ctrl_for_spawn.sleep_fn(),
                shutdown_signal,
            )
            .await;
        });

        let secs_since_last_sync = 45;
        let state = State {
            last_attempted_sync_at: Utc::now() - TimeDelta::seconds(secs_since_last_sync),
            last_synced_at: Utc::now(),
            cooldown_ends_at: Utc::now() - TimeDelta::seconds(10),
            err_streak: 0,
        };
        syncer.set_state(state);

        let syncer_tx = syncer.get_transmitter();

        let expected_sleep_secs = options.poll_interval_secs - secs_since_last_sync;
        let mut expected_num_sync_calls = 0; // only the first sync occurs
        for _ in 0..10 {
            expected_num_sync_calls += 1;
            sleep_ctrl.await_sleep().await;
            let last_attempted_sleep = sleep_ctrl.get_last_attempted_sleep().unwrap();
            assert!(last_attempted_sleep.as_secs() <= expected_sleep_secs as u64);
            assert!(last_attempted_sleep.as_secs() >= expected_sleep_secs as u64 - 1);
            assert_eq!(syncer.num_sync_calls(), expected_num_sync_calls);
            syncer_tx
                .send(SyncEvent::CooldownEnd(CooldownEnd::SyncFailure))
                .unwrap();
        }
    }
}
