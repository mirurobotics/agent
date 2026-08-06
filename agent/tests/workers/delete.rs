// standard crates
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::mocks::{deleter::MockDeleter, error::SleepController};
use miru_agent::delete::errors::QueueFullErr;
use miru_agent::delete::DeleteErr;
use miru_agent::trace;
use miru_agent::workers::delete;

// external crates
use tokio::sync::oneshot;

pub mod run {
    use super::*;

    fn sweep_err() -> DeleteErr {
        DeleteErr::QueueFullErr(QueueFullErr {
            capacity: 0,
            file: "mock sweep failure".to_string(),
            trace: trace!(),
        })
    }

    #[tokio::test]
    async fn drives_sweep_on_each_tick() {
        let options = delete::Options::default();
        let deleter = MockDeleter::new();
        let sleep_ctrl = Arc::new(SleepController::new());

        let options_for_spawn = options.clone();
        let deleter_for_spawn = deleter.clone();
        let sleep_ctrl_for_spawn = sleep_ctrl.clone();
        let shutdown_signal = Box::pin(async move {
            std::future::pending::<()>().await;
        });
        let _handle = tokio::spawn(async move {
            delete::run(
                &options_for_spawn,
                deleter_for_spawn.as_ref(),
                sleep_ctrl_for_spawn.sleep_fn(),
                shutdown_signal,
            )
            .await;
        });

        // the initial sweep runs immediately, before the first sleep
        sleep_ctrl.await_sleep().await;
        assert_eq!(deleter.num_sweep_calls(), 1);
        assert_eq!(
            sleep_ctrl.get_last_attempted_sleep().unwrap(),
            Duration::from_secs(options.sweep_interval_secs as u64)
        );

        // each released sleep drives exactly one more sweep on the fixed cadence
        for i in 0..10 {
            sleep_ctrl.release().await;
            sleep_ctrl.await_sleep().await;
            assert_eq!(deleter.num_sweep_calls(), i + 2);
            assert_eq!(
                sleep_ctrl.get_last_attempted_sleep().unwrap(),
                Duration::from_secs(options.sweep_interval_secs as u64)
            );
        }
    }

    #[tokio::test]
    async fn survives_sweep_error_and_continues() {
        let options = delete::Options::default();
        let deleter = MockDeleter::new();
        let sleep_ctrl = Arc::new(SleepController::new());

        let options_for_spawn = options.clone();
        let deleter_for_spawn = deleter.clone();
        let sleep_ctrl_for_spawn = sleep_ctrl.clone();
        let shutdown_signal = Box::pin(async move {
            std::future::pending::<()>().await;
        });
        let handle = tokio::spawn(async move {
            delete::run(
                &options_for_spawn,
                deleter_for_spawn.as_ref(),
                sleep_ctrl_for_spawn.sleep_fn(),
                shutdown_signal,
            )
            .await;
        });

        sleep_ctrl.await_sleep().await;
        assert_eq!(deleter.num_sweep_calls(), 1);

        // sweep() failures are logged and ignored; the loop keeps driving sweeps
        deleter.set_sweep(|| Err(sweep_err()));
        for i in 0..5 {
            sleep_ctrl.release().await;
            sleep_ctrl.await_sleep().await;
            assert_eq!(deleter.num_sweep_calls(), i + 2);
        }
        assert!(
            !handle.is_finished(),
            "worker must keep running through sweep() errors"
        );

        // and keeps driving sweeps once sweep() succeeds again
        deleter.set_sweep(|| Ok(()));
        for i in 0..5 {
            sleep_ctrl.release().await;
            sleep_ctrl.await_sleep().await;
            assert_eq!(deleter.num_sweep_calls(), i + 7);
        }
        assert!(!handle.is_finished());
    }

    #[tokio::test]
    async fn shuts_down_cleanly() {
        let options = delete::Options::default();
        let deleter = MockDeleter::new();
        let sleep_ctrl = Arc::new(SleepController::new());

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let options_for_spawn = options.clone();
        let deleter_for_spawn = deleter.clone();
        let sleep_ctrl_for_spawn = sleep_ctrl.clone();
        let shutdown_signal = Box::pin(async move {
            let _ = shutdown_rx.await;
        });
        let handle = tokio::spawn(async move {
            delete::run(
                &options_for_spawn,
                deleter_for_spawn.as_ref(),
                sleep_ctrl_for_spawn.sleep_fn(),
                shutdown_signal,
            )
            .await;
        });

        // the initial sweep ran and the worker is parked in its first sleep
        sleep_ctrl.await_sleep().await;
        let calls_before_shutdown = deleter.num_sweep_calls();
        assert_eq!(calls_before_shutdown, 1);

        // fire the shutdown; the worker completes without sweeping again
        shutdown_tx.send(()).unwrap();
        handle.await.unwrap();
        assert_eq!(deleter.num_sweep_calls(), calls_before_shutdown);
    }
}
