// standard crates
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::mocks::{error::SleepController, scanner::MockScanner};
use miru_agent::scan::errors::InternalError;
use miru_agent::scan::ScanErr;
use miru_agent::trace;
use miru_agent::workers::scan;

// external crates
use chrono::Utc;
use tokio::sync::oneshot;

pub mod run {
    use super::*;

    fn scan_err() -> ScanErr {
        ScanErr::InternalError(InternalError {
            message: "mock scan failure".to_string(),
            trace: trace!(),
        })
    }

    #[tokio::test]
    async fn drives_scan_on_each_tick() {
        let options = scan::Options::default();
        let scanner = Arc::new(MockScanner::default());
        let sleep_ctrl = Arc::new(SleepController::new());

        let options_for_spawn = options.clone();
        let scanner_for_spawn = scanner.clone();
        let sleep_ctrl_for_spawn = sleep_ctrl.clone();
        let shutdown_signal = Box::pin(async move {
            std::future::pending::<()>().await;
        });
        let _handle = tokio::spawn(async move {
            scan::run(
                &options_for_spawn,
                scanner_for_spawn.as_ref(),
                sleep_ctrl_for_spawn.sleep_fn(),
                shutdown_signal,
            )
            .await;
        });

        // the initial scan runs immediately, before the first sleep
        sleep_ctrl.await_sleep().await;
        assert_eq!(scanner.num_scan_calls(), 1);
        assert_eq!(
            sleep_ctrl.get_last_attempted_sleep().unwrap(),
            Duration::from_secs(options.scan_interval_secs as u64)
        );

        // each released sleep drives exactly one more scan on the fixed cadence
        for i in 0..10 {
            sleep_ctrl.release().await;
            sleep_ctrl.await_sleep().await;
            assert_eq!(scanner.num_scan_calls(), i + 2);
            assert_eq!(
                sleep_ctrl.get_last_attempted_sleep().unwrap(),
                Duration::from_secs(options.scan_interval_secs as u64)
            );
        }
    }

    #[tokio::test]
    async fn survives_scan_error_and_continues() {
        let options = scan::Options::default();
        let scanner = Arc::new(MockScanner::default());
        let sleep_ctrl = Arc::new(SleepController::new());

        let options_for_spawn = options.clone();
        let scanner_for_spawn = scanner.clone();
        let sleep_ctrl_for_spawn = sleep_ctrl.clone();
        let shutdown_signal = Box::pin(async move {
            std::future::pending::<()>().await;
        });
        let handle = tokio::spawn(async move {
            scan::run(
                &options_for_spawn,
                scanner_for_spawn.as_ref(),
                sleep_ctrl_for_spawn.sleep_fn(),
                shutdown_signal,
            )
            .await;
        });

        sleep_ctrl.await_sleep().await;
        assert_eq!(scanner.num_scan_calls(), 1);

        // scan() failures are logged and ignored; the loop keeps driving scans
        scanner.set_scan(|| Err(scan_err()));
        for i in 0..5 {
            sleep_ctrl.release().await;
            sleep_ctrl.await_sleep().await;
            assert_eq!(scanner.num_scan_calls(), i + 2);
        }
        assert!(
            !handle.is_finished(),
            "worker must keep running through scan() errors"
        );

        // and keeps driving scans once scan() succeeds again
        scanner.set_scan(|| Ok(()));
        for i in 0..5 {
            sleep_ctrl.release().await;
            sleep_ctrl.await_sleep().await;
            assert_eq!(scanner.num_scan_calls(), i + 7);
        }
        assert!(!handle.is_finished());
    }

    #[tokio::test]
    async fn prunes_after_each_scan_tick() {
        let options = scan::Options::default();
        let retention = chrono::Duration::seconds(options.ledger_retention_secs);
        let scanner = Arc::new(MockScanner::default());
        let sleep_ctrl = Arc::new(SleepController::new());

        let t0 = Utc::now();
        let options_for_spawn = options.clone();
        let scanner_for_spawn = scanner.clone();
        let sleep_ctrl_for_spawn = sleep_ctrl.clone();
        let shutdown_signal = Box::pin(async move {
            std::future::pending::<()>().await;
        });
        let _handle = tokio::spawn(async move {
            scan::run(
                &options_for_spawn,
                scanner_for_spawn.as_ref(),
                sleep_ctrl_for_spawn.sleep_fn(),
                shutdown_signal,
            )
            .await;
        });

        // the initial scan is followed by exactly one prune
        sleep_ctrl.await_sleep().await;
        assert_eq!(scanner.num_scan_calls(), 1);
        assert_eq!(scanner.prune_calls().len(), 1);

        // each subsequent tick drives one scan and one prune
        for i in 0..3 {
            sleep_ctrl.release().await;
            sleep_ctrl.await_sleep().await;
            assert_eq!(scanner.num_scan_calls(), i + 2);
            assert_eq!(scanner.prune_calls().len(), i + 2);
        }

        // every prune cutoff is now - retention (bounded by the test window)
        for before in scanner.prune_calls() {
            assert!(before >= t0 - retention);
            assert!(before <= Utc::now() - retention);
        }
    }

    #[tokio::test]
    async fn survives_prune_error_and_continues() {
        let options = scan::Options::default();
        let scanner = Arc::new(MockScanner::default());
        let sleep_ctrl = Arc::new(SleepController::new());

        let options_for_spawn = options.clone();
        let scanner_for_spawn = scanner.clone();
        let sleep_ctrl_for_spawn = sleep_ctrl.clone();
        let shutdown_signal = Box::pin(async move {
            std::future::pending::<()>().await;
        });
        let handle = tokio::spawn(async move {
            scan::run(
                &options_for_spawn,
                scanner_for_spawn.as_ref(),
                sleep_ctrl_for_spawn.sleep_fn(),
                shutdown_signal,
            )
            .await;
        });

        sleep_ctrl.await_sleep().await;
        assert_eq!(scanner.num_scan_calls(), 1);
        assert_eq!(scanner.prune_calls().len(), 1);

        // prune() failures are logged and ignored; scans and prunes keep coming
        scanner.set_prune(|| Err(scan_err()));
        for i in 0..5 {
            sleep_ctrl.release().await;
            sleep_ctrl.await_sleep().await;
            assert_eq!(scanner.num_scan_calls(), i + 2);
            assert_eq!(scanner.prune_calls().len(), i + 2);
        }
        assert!(
            !handle.is_finished(),
            "worker must keep running through prune() errors"
        );

        // and keeps driving once prune() succeeds again
        scanner.set_prune(|| Ok(()));
        for i in 0..5 {
            sleep_ctrl.release().await;
            sleep_ctrl.await_sleep().await;
            assert_eq!(scanner.num_scan_calls(), i + 7);
            assert_eq!(scanner.prune_calls().len(), i + 7);
        }
        assert!(!handle.is_finished());
    }

    #[tokio::test]
    async fn shuts_down_cleanly() {
        let options = scan::Options::default();
        let scanner = Arc::new(MockScanner::default());
        let sleep_ctrl = Arc::new(SleepController::new());

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let options_for_spawn = options.clone();
        let scanner_for_spawn = scanner.clone();
        let sleep_ctrl_for_spawn = sleep_ctrl.clone();
        let shutdown_signal = Box::pin(async move {
            let _ = shutdown_rx.await;
        });
        let handle = tokio::spawn(async move {
            scan::run(
                &options_for_spawn,
                scanner_for_spawn.as_ref(),
                sleep_ctrl_for_spawn.sleep_fn(),
                shutdown_signal,
            )
            .await;
        });

        // the initial scan ran and the worker is parked in its first sleep
        sleep_ctrl.await_sleep().await;
        let calls_before_shutdown = scanner.num_scan_calls();
        assert_eq!(calls_before_shutdown, 1);

        // fire the shutdown; the worker completes without scanning again
        shutdown_tx.send(()).unwrap();
        handle.await.unwrap();
        assert_eq!(scanner.num_scan_calls(), calls_before_shutdown);
    }
}
