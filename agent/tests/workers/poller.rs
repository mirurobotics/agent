// standard crates
use std::sync::Arc;

// internal crates
use miru_agent::filesys::dir::Dir;
use miru_agent::models::device::Device;
use miru_agent::storage::{device::DeviceFile, layout::StorageLayout};
use miru_agent::sync::{
    errors::{MockErr as SyncMockErr, SyncErr},
    syncer::{CooldownEnd, SyncEvent, SyncFailure, SyncState},
};
use miru_agent::workers::poller;

use crate::mock::SleepController;
use crate::sync::mock::MockSyncer;

// external crates
use chrono::{TimeDelta, Utc};

pub mod run {
    use super::*;

    #[tokio::test]
    async fn syncer_not_in_cooldown() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let layout = StorageLayout::new(dir);

        let (device_file, _) =
            DeviceFile::spawn_with_default(64, layout.device_file(), Device::default())
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
        let state = SyncState {
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
                is_network_connection_error: true,
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
                is_network_connection_error: false,
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
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let layout = StorageLayout::new(dir);

        let (device_file, _) =
            DeviceFile::spawn_with_default(64, layout.device_file(), Device::default())
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
        let state = SyncState {
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
                is_network_connection_error: true,
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
                is_network_connection_error: false,
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
    async fn ignored_syncer_events() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let layout = StorageLayout::new(dir);

        let (device_file, _) =
            DeviceFile::spawn_with_default(64, layout.device_file(), Device::default())
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
        let state = SyncState {
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
                is_network_connection_error: true,
            }),
            SyncEvent::CooldownEnd(CooldownEnd::FromSyncSuccess),
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
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let layout = StorageLayout::new(dir);

        let (device_file, _) =
            DeviceFile::spawn_with_default(64, layout.device_file(), Device::default())
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
        let state = SyncState {
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
                .send(SyncEvent::CooldownEnd(CooldownEnd::FromSyncFailure))
                .unwrap();
        }
    }
}
