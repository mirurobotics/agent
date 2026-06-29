// standard crates
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

// internal crates
use crate::upload::UploaderExt;

// external crates
use tracing::info;

#[derive(Debug, Clone)]
pub struct Options {
    /// Base tick interval between `scan()` passes. The uploader internally skips
    /// rules whose `next_scan_at` has not elapsed, so the worker is pure timing.
    pub tick_interval_secs: i64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            tick_interval_secs: 1,
        }
    }
}

pub async fn run<F, Fut, UploaderT: UploaderExt>(
    options: &Options,
    uploader: &UploaderT,
    sleep_fn: F,
    mut shutdown_signal: Pin<Box<impl Future<Output = ()> + Send + 'static>>,
) where
    F: Fn(Duration) -> Fut,
    Fut: Future<Output = ()> + Send,
{
    tokio::select! {
        _ = shutdown_signal.as_mut() => {
            info!("Uploads worker shutdown complete");
        }
        // doesn't return but we do need to run it in the background
        _ = run_impl(options, uploader, sleep_fn) => {}
    }
}

async fn run_impl<F, Fut, UploaderT: UploaderExt>(
    options: &Options,
    uploader: &UploaderT,
    sleep_fn: F, // for testing purposes
) where
    F: Fn(Duration) -> Fut,
    Fut: Future<Output = ()> + Send,
{
    info!("Running uploads worker");

    loop {
        let _ = uploader.scan().await;
        sleep_fn(Duration::from_secs(options.tick_interval_secs.max(1) as u64)).await;
    }
}
