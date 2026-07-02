// standard crates
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

// internal crates
use crate::upload::{self, UploaderExt};

// external crates
use tracing::{error, info};

pub async fn run<F, Fut, UploaderT: UploaderExt>(
    options: &upload::Options,
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

/// Ticks on the uploader's global poll interval and triggers a scan per tick.
/// All scheduling and scan state lives in the uploader; this worker is timing
/// only.
async fn run_impl<F, Fut, UploaderT: UploaderExt>(
    options: &upload::Options,
    uploader: &UploaderT,
    sleep_fn: F, // for testing purposes
) where
    F: Fn(Duration) -> Fut,
    Fut: Future<Output = ()> + Send,
{
    info!("Running uploads worker");

    loop {
        if let Err(e) = uploader.scan().await {
            error!("failed to scan for upload-ready files: {e:?}");
        }
        sleep_fn(Duration::from_secs(options.poll_interval_secs.max(0) as u64)).await;
    }
}
