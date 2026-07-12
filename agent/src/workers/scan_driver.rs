// This module is named `scan_driver` (not `scan`) to avoid aliasing the
// `crate::scan` subsystem module: it names its role — it *drives* the scanner
// actor, which is reactive and not self-scheduling, on a fixed cadence.

// standard crates
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

// internal crates
use crate::scan::ScannerExt;

// external crates
use tracing::{debug, error, info};

#[derive(Debug, Clone)]
pub struct Options {
    pub scan_interval_secs: i64,
}

impl Default for Options {
    fn default() -> Self {
        // scanning is a cheap local filesystem discover/evaluate pass (no
        // network) and rule stability windows are second-to-minute scale, so a
        // 1-minute cadence keeps newly-stable files flowing promptly without
        // busy-looping.
        Self {
            scan_interval_secs: 60,
        }
    }
}

pub async fn run<F, Fut, ScannerT: ScannerExt>(
    options: &Options,
    scanner: &ScannerT,
    sleep_fn: F,
    mut shutdown_signal: Pin<Box<impl Future<Output = ()> + Send + 'static>>,
) where
    F: Fn(Duration) -> Fut,
    Fut: Future<Output = ()> + Send,
{
    tokio::select! {
        _ = shutdown_signal.as_mut() => {
            info!("Scan driver worker shutdown complete");
        }
        // doesn't return but we do need to run it in the background
        _ = run_impl(options, scanner, sleep_fn) => {}
    }
}

async fn run_impl<F, Fut, ScannerT: ScannerExt>(
    options: &Options,
    scanner: &ScannerT,
    sleep_fn: F, // for testing purposes
) where
    F: Fn(Duration) -> Fut,
    Fut: Future<Output = ()> + Send,
{
    info!("Running scan driver worker");

    // drive an initial scan immediately, then on a fixed cadence
    if let Err(e) = scanner.scan().await {
        error!("scan driver: initial scan failed: {e:?}");
    }

    let interval = Duration::from_secs(options.scan_interval_secs.max(0) as u64);
    loop {
        debug!("scan driver: sleeping {interval:?} until next scan");
        sleep_fn(interval).await;
        if let Err(e) = scanner.scan().await {
            error!("scan driver: scan failed, continuing: {e:?}");
        }
    }
}
