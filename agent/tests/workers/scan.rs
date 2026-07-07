// standard crates
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

// internal crates
use crate::mocks::error::SleepController;
use miru_agent::models::{Deployment, UploadRule};
use miru_agent::scan::{Config, ScannerExt, ScanErr};
use miru_agent::workers::scan;

// A counting mock implementing ScannerExt: records how many times scan() is
// called so the thin timing worker can be observed without any real scanner.
struct CountingScanner {
    scans: Arc<AtomicUsize>,
}

impl ScannerExt for CountingScanner {
    async fn update_configs(&self, _cfgs: Vec<Config>) -> Result<(), ScanErr> {
        Ok(())
    }
    async fn scan(&self) -> Result<(), ScanErr> {
        self.scans.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn subscribe(
        &self,
    ) -> Result<tokio::sync::broadcast::Receiver<miru_agent::scan::ScanEvent>, ScanErr> {
        let (_tx, rx) = tokio::sync::broadcast::channel(1);
        Ok(rx)
    }
    async fn shutdown(&self) -> Result<(), ScanErr> {
        Ok(())
    }
}

// The worker scans once per tick: scan() then sleep(tick_interval). Each
// release of the controlled sleep advances exactly one tick.
#[tokio::test]
async fn scan_called_each_tick() {
    let options = scan::Options {
        tick_interval_secs: 1,
    };
    let sleep_ctrl = Arc::new(SleepController::new());
    let scans = Arc::new(AtomicUsize::new(0));
    let scanner = Arc::new(CountingScanner {
        scans: scans.clone(),
    });

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

    // first tick: scan() runs, then the worker reaches its sleep.
    sleep_ctrl.await_sleep().await;
    assert_eq!(scans.load(Ordering::SeqCst), 1);
    assert_eq!(sleep_ctrl.get_last_attempted_sleep().unwrap().as_secs(), 1);

    // each release advances exactly one tick (one additional scan).
    for i in 0..5 {
        sleep_ctrl.release().await;
        sleep_ctrl.await_sleep().await;
        assert_eq!(scans.load(Ordering::SeqCst), i + 2);
        assert_eq!(sleep_ctrl.get_last_attempted_sleep().unwrap().as_secs(), 1);
    }
}

// The worker's run future returns once the shutdown signal fires.
#[tokio::test]
async fn shuts_down_on_signal() {
    let options = scan::Options {
        tick_interval_secs: 1,
    };
    let sleep_ctrl = Arc::new(SleepController::new());
    let scans = Arc::new(AtomicUsize::new(0));
    let scanner = Arc::new(CountingScanner {
        scans: scans.clone(),
    });

    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let shutdown_signal = Box::pin(async move {
        let _ = rx.await;
    });

    let options_for_spawn = options.clone();
    let scanner_for_spawn = scanner.clone();
    let sleep_ctrl_for_spawn = sleep_ctrl.clone();
    let handle = tokio::spawn(async move {
        scan::run(
            &options_for_spawn,
            scanner_for_spawn.as_ref(),
            sleep_ctrl_for_spawn.sleep_fn(),
            shutdown_signal,
        )
        .await;
    });

    // let the worker reach its first sleep so it is actively running.
    sleep_ctrl.await_sleep().await;

    // trigger shutdown; the run future should select the shutdown branch and
    // return, completing the task.
    tx.send(()).unwrap();
    handle.await.unwrap();
}
