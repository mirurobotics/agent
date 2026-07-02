// standard crates
use std::sync::Arc;

// internal crates
use crate::mocks::{error::SleepController, uploader::MockUploader};
use miru_agent::trace;
use miru_agent::upload::{self, errors::SendActorMessageErr, UploadErr};
use miru_agent::workers::uploads;

// external crates
use tokio::sync::broadcast;

fn spawn_worker(
    options: upload::Options,
    uploader: Arc<MockUploader>,
    sleep_ctrl: Arc<SleepController>,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        uploads::run(
            &options,
            uploader.as_ref(),
            sleep_ctrl.sleep_fn(),
            Box::pin(async move {
                let _ = shutdown_rx.recv().await;
            }),
        )
        .await;
    })
}

pub mod run {
    use super::*;

    #[tokio::test]
    async fn one_scan_per_tick() {
        let options = upload::Options {
            poll_interval_secs: 42,
        };
        let uploader = Arc::new(MockUploader::default());
        let sleep_ctrl = Arc::new(SleepController::new());
        let (_shutdown_tx, shutdown_rx) = broadcast::channel::<()>(1);

        let _handle = spawn_worker(
            options.clone(),
            uploader.clone(),
            sleep_ctrl.clone(),
            shutdown_rx,
        );

        for i in 0..10 {
            sleep_ctrl.await_sleep().await;
            let last_sleep = sleep_ctrl.get_last_attempted_sleep().unwrap();
            assert_eq!(last_sleep.as_secs(), options.poll_interval_secs as u64);
            assert_eq!(uploader.num_scan_calls(), i + 1);
            sleep_ctrl.release().await;
        }
    }

    #[tokio::test]
    async fn scan_errors_are_logged_and_ignored() {
        let options = upload::Options {
            poll_interval_secs: 42,
        };
        let uploader = Arc::new(MockUploader::default());
        uploader.set_scan(|| {
            Err(UploadErr::SendActorMessageErr(SendActorMessageErr {
                source: "scan failed".into(),
                trace: trace!(),
            }))
        });
        let sleep_ctrl = Arc::new(SleepController::new());
        let (_shutdown_tx, shutdown_rx) = broadcast::channel::<()>(1);

        let _handle = spawn_worker(
            options.clone(),
            uploader.clone(),
            sleep_ctrl.clone(),
            shutdown_rx,
        );

        for i in 0..10 {
            sleep_ctrl.await_sleep().await;
            assert_eq!(uploader.num_scan_calls(), i + 1);
            sleep_ctrl.release().await;
        }
    }

    #[tokio::test]
    async fn shutdown() {
        let options = upload::Options::default();
        let uploader = Arc::new(MockUploader::default());
        let sleep_ctrl = Arc::new(SleepController::new());
        let (shutdown_tx, shutdown_rx) = broadcast::channel::<()>(1);

        let handle = spawn_worker(options, uploader.clone(), sleep_ctrl.clone(), shutdown_rx);

        // wait for the first scan to complete and the worker to enter its sleep
        sleep_ctrl.await_sleep().await;
        assert_eq!(uploader.num_scan_calls(), 1);

        shutdown_tx.send(()).unwrap();
        handle.await.unwrap();
        assert_eq!(uploader.num_scan_calls(), 1);
    }
}
