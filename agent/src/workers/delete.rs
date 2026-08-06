// External driver for the `crate::delete` deleter actor. The actor is reactive,
// not self-scheduling: this worker imposes the cadence that drives repeated
// sweep passes.

// standard crates
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

// internal crates
use crate::delete::DeleterExt;

// external crates
use tracing::{debug, error, info};

#[derive(Debug, Clone)]
pub struct Options {
    pub sweep_interval_secs: i64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            sweep_interval_secs: 60,
        }
    }
}

pub async fn run<F, Fut, DeleterT: DeleterExt>(
    options: &Options,
    deleter: &DeleterT,
    sleep_fn: F,
    mut shutdown_signal: Pin<Box<impl Future<Output = ()> + Send + 'static>>,
) where
    F: Fn(Duration) -> Fut,
    Fut: Future<Output = ()> + Send,
{
    tokio::select! {
        _ = shutdown_signal.as_mut() => {
            info!("delete: driver worker shutdown complete");
        }
        // doesn't return but we do need to run it in the background
        _ = run_impl(options, deleter, sleep_fn) => {}
    }
}

async fn run_impl<F, Fut, DeleterT: DeleterExt>(
    options: &Options,
    deleter: &DeleterT,
    sleep_fn: F, // for testing purposes
) where
    F: Fn(Duration) -> Fut,
    Fut: Future<Output = ()> + Send,
{
    info!("delete: running driver worker");

    // drive an initial sweep immediately, then on a fixed cadence, so entries
    // that came due while the agent was down are processed promptly on restart
    if let Err(e) = deleter.sweep().await {
        error!("delete: initial sweep failed: {e:?}");
    }

    let interval = Duration::from_secs(options.sweep_interval_secs.max(0) as u64);
    loop {
        debug!("delete: sleeping {interval:?} until next sweep");
        sleep_fn(interval).await;
        if let Err(e) = deleter.sweep().await {
            error!("delete: sweep failed, continuing: {e:?}");
        }
    }
}
