// standard crates
use std::sync::Arc;

// internal crates
use miru_agent::filesys::File;
use miru_agent::models::{Deployment, UploadRule};
use miru_agent::scan::scanner::StableFile;
use miru_agent::scan::{ScanErr, ScanEvent, ScannerExt};
use miru_agent::workers::scan_stable;

// external crates
use chrono::{DateTime, Utc};
use tokio::sync::broadcast;

// =============================== TEST HELPERS ================================= //

/// A fake ScannerExt for the stable-file sink worker. `subscribe()` either returns
/// a receiver bound to a test-owned broadcast sender or an error, depending on
/// `subscribe_fails`.
struct FakeScanner {
    tx: broadcast::Sender<ScanEvent>,
    subscribe_fails: bool,
    // when true, subscribe() returns a receiver whose only sender is already
    // dropped, so the first recv() yields RecvError::Closed.
    pre_closed: bool,
}

impl FakeScanner {
    fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self {
            tx,
            subscribe_fails: false,
            pre_closed: false,
        }
    }

    fn failing() -> Self {
        let (tx, _rx) = broadcast::channel(1);
        Self {
            tx,
            subscribe_fails: true,
            pre_closed: false,
        }
    }

    fn pre_closed() -> Self {
        let (tx, _rx) = broadcast::channel(1);
        Self {
            tx,
            subscribe_fails: false,
            pre_closed: true,
        }
    }
}

impl ScannerExt for FakeScanner {
    async fn clear_rules(&self) -> Result<(), ScanErr> {
        Ok(())
    }
    async fn update_rules(
        &self,
        _deployment: Deployment,
        _rules: Vec<UploadRule>,
    ) -> Result<(), ScanErr> {
        Ok(())
    }
    async fn scan(&self) -> Result<(), ScanErr> {
        Ok(())
    }
    async fn subscribe(&self) -> Result<broadcast::Receiver<ScanEvent>, ScanErr> {
        if self.subscribe_fails {
            return Err(ScanErr::InternalError(
                miru_agent::scan::errors::InternalError {
                    message: "subscribe failed".to_string(),
                    trace: miru_agent::trace!(),
                },
            ));
        }
        if self.pre_closed {
            // fresh channel whose only sender is dropped immediately => the
            // receiver's first recv() returns Closed.
            let (tx, rx) = broadcast::channel(1);
            drop(tx);
            return Ok(rx);
        }
        Ok(self.tx.subscribe())
    }
    async fn shutdown(&self) -> Result<(), ScanErr> {
        Ok(())
    }
    async fn prune(&self, _before: DateTime<Utc>) -> Result<(), ScanErr> {
        Ok(())
    }
}

fn stable_file(name: &str) -> StableFile {
    StableFile {
        file: File::new(format!("/tmp/{name}")),
        size: 3,
        digest: "digest".to_string(),
        mtime: DateTime::<Utc>::UNIX_EPOCH,
        first_observed_at: DateTime::<Utc>::UNIX_EPOCH,
        last_observed_at: DateTime::<Utc>::UNIX_EPOCH,
        deployment_id: "dpl".to_string(),
        upload_rule_id: "rule".to_string(),
    }
}

// ================================== TESTS ==================================== //

// A StableFile event is consumed by the worker: the run future keeps running (does
// not return) after the event, so a shutdown signal is required to end it.
#[tokio::test]
async fn consumes_stable_file_events() {
    let scanner = Arc::new(FakeScanner::new(16));
    let tx = scanner.tx.clone();

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let shutdown_signal = Box::pin(async move {
        let _ = shutdown_rx.await;
    });

    let scanner_for_spawn = scanner.clone();
    let handle = tokio::spawn(async move {
        scan_stable::run(scanner_for_spawn.as_ref(), shutdown_signal).await;
    });

    // give the worker a moment to subscribe, then send an event.
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    tx.send(ScanEvent::StableFile(stable_file("a.mcap")))
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    // the worker is still running (consumed the event, did not exit): shut it down.
    shutdown_tx.send(()).unwrap();
    handle.await.unwrap();
}

// When the broadcast sender is dropped, recv() returns Closed and run_impl breaks,
// so run(...) returns even without a shutdown signal.
#[tokio::test]
async fn closed_channel_returns() {
    let scanner = FakeScanner::pre_closed();

    scan_stable::run(
        &scanner,
        Box::pin(async move { std::future::pending::<()>().await }),
    )
    .await;
    // subscribe() returned a receiver with no live sender, so the first recv()
    // yields Closed, run_impl breaks and run returns. Reaching here proves it.
}

// A lagged subscriber (more events than capacity, undrained) hits the Lagged warn
// branch and keeps looping rather than exiting.
#[tokio::test]
async fn lagged_subscriber_keeps_running() {
    let scanner = Arc::new(FakeScanner::new(1));
    let tx = scanner.tx.clone();

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let shutdown_signal = Box::pin(async move {
        let _ = shutdown_rx.await;
    });

    let scanner_for_spawn = scanner.clone();
    let handle = tokio::spawn(async move {
        scan_stable::run(scanner_for_spawn.as_ref(), shutdown_signal).await;
    });

    // flood the capacity-1 channel before the worker drains it.
    for i in 0..8 {
        let _ = tx.send(ScanEvent::StableFile(stable_file(&format!("f{i}.mcap"))));
    }
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;

    // still running after the lag: shut it down cleanly.
    shutdown_tx.send(()).unwrap();
    handle.await.unwrap();
}

// If subscribe() fails, run_impl warns and returns early; run(...) returns.
#[tokio::test]
async fn subscribe_failure_returns_early() {
    let scanner = FakeScanner::failing();
    scan_stable::run(
        &scanner,
        Box::pin(async move { std::future::pending::<()>().await }),
    )
    .await;
    // subscribe returned Err, so run_impl returned immediately and run returned.
}

// The worker returns when the shutdown signal fires (select! shutdown arm).
#[tokio::test]
async fn shuts_down_on_signal() {
    let scanner = Arc::new(FakeScanner::new(16));

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let shutdown_signal = Box::pin(async move {
        let _ = shutdown_rx.await;
    });

    let scanner_for_spawn = scanner.clone();
    let handle = tokio::spawn(async move {
        scan_stable::run(scanner_for_spawn.as_ref(), shutdown_signal).await;
    });

    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    shutdown_tx.send(()).unwrap();
    handle.await.unwrap();
}
