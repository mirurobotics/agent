// standard crates
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

// internal crates
use crate::scan::ScannerExt;

// external crates
use tracing::info;

#[derive(Debug, Clone)]
pub struct Options {
    pub poll_interval_secs: i64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            poll_interval_secs: 15,
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
            info!("Scan worker shutdown complete");
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
    info!("Running scan worker");

    loop {
        let _ = scanner.scan().await;
        sleep_fn(Duration::from_secs(options.poll_interval_secs.max(1) as u64)).await;
    }
}
