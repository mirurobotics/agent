// External driver for the `crate::scan` scanner actor. The actor is reactive,
// not self-scheduling: this worker imposes the cadence that drives repeated
// scan passes.

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
