// standard crates
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

// internal crates
use crate::disk;
use crate::models::device;
use crate::sync::{
    syncer::{CooldownEnd, SyncEvent},
    SyncerExt,
};
use crate::workers::next_sync_event;

// external crates
use chrono::Utc;
use tracing::{debug, error, info};

#[derive(Debug, Clone)]
pub struct Options {
    pub poll_interval_secs: i64,
}

impl Default for Options {
    fn default() -> Self {
        let twelve_hours = 12 * 60 * 60;
        Self {
            poll_interval_secs: twelve_hours,
        }
    }
}

pub async fn run<F, Fut, SyncerT: SyncerExt>(
    options: &Options,
    syncer: &SyncerT,
    device_stor: &disk::Device,
    sleep_fn: F,
    mut shutdown_signal: Pin<Box<impl Future<Output = ()> + Send + 'static>>,
) where
    F: Fn(Duration) -> Fut,
    Fut: Future<Output = ()> + Send,
{
    tokio::select! {
        _ = shutdown_signal.as_mut() => {
            info!("Poller worker shutdown complete");
        }
        // doesn't return but we do need to run it in the background
        _ = run_impl(
            options,
            syncer,
            device_stor,
            sleep_fn,
        ) => {}
    }
}

async fn run_impl<F, Fut, SyncerT: SyncerExt>(
    options: &Options,
    syncer: &SyncerT,
    device_stor: &disk::Device,
    sleep_fn: F, // for testing purposes
) where
    F: Fn(Duration) -> Fut,
    Fut: Future<Output = ()> + Send,
{
    info!("Running poller worker");

    let mut syncer_subscriber = match syncer.subscribe().await {
        Ok(subscriber) => Some(subscriber),
        Err(e) => {
            error!("error subscribing to syncer events: {e:?}; disabling event subscription");
            None
        }
    };

    // begin by syncing
    let _ = syncer.sync_if_not_in_cooldown().await;

    loop {
        let wait = next_wait(options, syncer).await;
        debug!("Waiting {wait:?} for next *scheduled* device sync");

        tokio::select! {
            // next scheduled sync
            _ = sleep_fn(wait) => {
                let _ = syncer.sync_if_not_in_cooldown().await;
            }

            // listen for syncer events from the syncer worker (this device)
            event = next_sync_event(&mut syncer_subscriber) => {
                match event {
                    Some(SyncEvent::CooldownEnd(
                        CooldownEnd::SyncFailure | CooldownEnd::DeploymentWait,
                    )) => {
                        let _ = syncer.sync_if_not_in_cooldown().await;
                    }
                    Some(SyncEvent::SyncSuccess) => {
                        let patch = device::Updates {
                            last_synced_at: Some(Utc::now()),
                            ..device::Updates::empty()
                        };
                        let _ = device_stor.patch(patch).await;
                    }
                    Some(_) => {}
                    None => {
                        error!("syncer event stream ended; disabling event subscription");
                        syncer_subscriber = None;
                    }
                }
            }
        }
    }
}

// time to wait before the next scheduled sync: the later of the poll interval
// (measured from the last sync attempt, not the current time) and the cooldown
// end. Both use the monotonic clock, so a system clock step cannot stretch or
// skip the wait.
async fn next_wait<SyncerT: SyncerExt>(options: &Options, syncer: &SyncerT) -> Duration {
    let poll_interval = Duration::from_secs(u64::try_from(options.poll_interval_secs).unwrap_or(0));
    let until_next_poll = match syncer.get_since_last_attempt().await {
        Ok(Some(since)) => poll_interval.saturating_sub(since),
        // no attempt yet in this process: poll now
        Ok(None) | Err(_) => Duration::ZERO,
    };
    let until_cooldown_ends = syncer.get_remaining_cooldown().await.unwrap_or_default();
    until_next_poll.max(until_cooldown_ends)
}
