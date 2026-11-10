// standard crates
use std::future::Future;
use std::pin::Pin;

// internal crates
use crate::authn::TokenManagerExt;
use crate::calibrations::{self, LatestStore};
use crate::http;
use crate::sync::{
    syncer::{next_sync_event, SyncEvent},
    SyncerExt,
};

// external crates
use tracing::{error, info, warn};

/// Merge Miru's latest calibrations into the latest store once per run, after
/// a successful sync, for the reports that an agent that doesn't write the
/// store (an older one, or one rolled back to) took. A failed pull is retried
/// after the next successful sync. Returns once a pull succeeds, or on
/// shutdown, which stops a pull midway.
pub async fn run<SyncerT, HTTPClientT, TokenManagerT>(
    syncer: &SyncerT,
    store: &LatestStore,
    http_client: &HTTPClientT,
    token_mngr: &TokenManagerT,
    mut shutdown_signal: Pin<Box<impl Future<Output = ()> + Send + 'static>>,
) where
    SyncerT: SyncerExt,
    HTTPClientT: http::ClientI,
    TokenManagerT: TokenManagerExt,
{
    tokio::select! {
        _ = shutdown_signal.as_mut() => {
            info!("calibrations pull worker shutdown complete");
        }
        _ = run_impl(syncer, store, http_client, token_mngr) => {}
    }
}

async fn run_impl<SyncerT, HTTPClientT, TokenManagerT>(
    syncer: &SyncerT,
    store: &LatestStore,
    http_client: &HTTPClientT,
    token_mngr: &TokenManagerT,
) where
    SyncerT: SyncerExt,
    HTTPClientT: http::ClientI,
    TokenManagerT: TokenManagerExt,
{
    // the seed isn't consumed, so a sync that ended before this subscribed
    // still starts a pull
    let mut subscriber = match syncer.subscribe().await {
        Ok(subscriber) => Some(subscriber),
        Err(e) => {
            error!("calibrations pull: error subscribing to syncer events: {e:?}");
            return;
        }
    };
    while let Some(event) = next_sync_event(&mut subscriber).await {
        if !matches!(event, SyncEvent::SyncSuccess) {
            continue;
        }
        let token = match token_mngr.get_token().await {
            Ok(token) => token,
            Err(e) => {
                warn!("calibrations pull: unable to get a token: {e}");
                continue;
            }
        };
        match calibrations::pull_latest(store, http_client, &token.token).await {
            Ok(()) => return,
            Err(e) => warn!("calibrations pull: unable to pull the latest calibrations: {e}"),
        }
    }
    error!("calibrations pull: syncer event stream ended before a pull succeeded");
}
