// standard crates
use std::cmp::max;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

// internal modules
use crate::models::device;
use crate::storage;
use crate::sync::syncer::{CooldownEnd, SyncEvent};
use crate::sync::SyncerExt;

// external crates
use chrono::{TimeDelta, Utc};
use tokio::sync::watch;
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
    device_stor: &storage::Device,
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
    device_stor: &storage::Device,
    sleep_fn: F, // for testing purposes
) where
    F: Fn(Duration) -> Fut,
    Fut: Future<Output = ()> + Send,
{
    info!("Running poller worker");

    // subscribe to syncer events
    let mut syncer_subscriber = syncer.subscribe().await.unwrap_or_else(|e| {
        error!("error subscribing to syncer events: {e:?}");
        // Create a dummy receiver that never sends anything
        watch::channel(SyncEvent::SyncSuccess).1
    });

    // begin by syncing
    let _ = syncer.sync_if_not_in_cooldown().await;

    loop {
        // poll from the last sync attempt, not the current time
        let last_attempted_sync_at = syncer
            .get_last_attempted_sync_at()
            .await
            .unwrap_or_default()
            .timestamp();
        let secs_since_last_sync = Utc::now().timestamp() - last_attempted_sync_at;
        let secs_until_next_sync = options.poll_interval_secs - secs_since_last_sync;

        // wait until the cooldown ends or the poll interval elapses (max of the two)
        let secs_until_cooldown_ends = syncer
            .get_cooldown_ends_at()
            .await
            .unwrap_or_default()
            .signed_duration_since(Utc::now())
            .num_seconds();
        let wait_secs = max(secs_until_next_sync, secs_until_cooldown_ends);

        // log the next scheduled sync time
        let next_sync_at = Utc::now() + TimeDelta::seconds(wait_secs);
        debug!(
            "Waiting until {:?} ({:?} seconds) for next *scheduled* device sync",
            next_sync_at, wait_secs
        );

        tokio::select! {
            // next scheduled sync
            _ = sleep_fn(Duration::from_secs(wait_secs as u64)) => {
                let _ = syncer.sync_if_not_in_cooldown().await;
            }

            // listen for syncer events from the syncer worker (this device)
            _ = syncer_subscriber.changed() => {
                let syncer_event = syncer_subscriber.borrow().clone();

                match &syncer_event {
                    SyncEvent::CooldownEnd(
                        CooldownEnd::SyncFailure | CooldownEnd::DeploymentWait,
                    ) => {
                        let _ = syncer.sync_if_not_in_cooldown().await;
                    }
                    SyncEvent::SyncSuccess => {
                        let patch = device::Updates {
                            last_synced_at: Some(Utc::now()),
                            ..device::Updates::empty()
                        };
                        let _ = device_stor.patch(patch).await;
                    }
                    _ => {}
                }
            }
        }
    }
}
