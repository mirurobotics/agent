// internal crates
use crate::app::errors::MetadataSyncErr;
use crate::authn;
use crate::disk::{self, Layout};
use crate::http::{self, ClientI};
use crate::models;
use crate::network::BackendHost;

// external crates
use tracing::{debug, warn};

/// Outcome of a successful [`sync_system_metadata`] run.
#[derive(Debug, PartialEq, Eq)]
pub enum Synced {
    /// Live metadata matched the cache; no device-id resolution and no network
    /// call were performed.
    Unchanged,
    /// Live metadata differed from the cache (or the cache was absent); the
    /// backend device was PATCHed and the cache refreshed.
    Updated,
}

/// Compare the running host's system metadata against the on-disk cache and,
/// only when they differ (or the cache is absent/unreadable), PATCH the backend
/// device with the metadata fields and refresh the cache.
///
/// The cache is written **only after** a confirmed successful PATCH, so a failed
/// PATCH returns `Err` before the cache is touched and the sync is retried on
/// the next boot. A missing or unreadable cache is treated as "no cache" (the
/// read error is logged, never propagated).
pub async fn sync_system_metadata(
    http_client: &impl ClientI,
    layout: &Layout,
    token: &str,
) -> Result<Synced, MetadataSyncErr> {
    let live = models::system_metadata();
    let cache_file = layout.system_metadata();

    let cached = match disk::system_metadata::read(&cache_file).await {
        Ok(cached) => cached,
        Err(e) => {
            warn!("unable to read system-metadata cache; treating as absent: {e}");
            None
        }
    };
    if cached.as_ref() == Some(&live) {
        debug!("system metadata unchanged since last boot; skipping device update");
        return Ok(Synced::Unchanged);
    }

    let id = disk::resolve_device_id(layout).await?;
    let payload = live.to_update_request();
    http::devices::update(
        http_client,
        http::devices::UpdateParams {
            id: &id,
            payload: &payload,
            token,
        },
    )
    .await?;

    disk::system_metadata::write(&cache_file, &live).await?;
    Ok(Synced::Updated)
}

/// Best-effort boot-time wrapper around [`sync_system_metadata`]. Builds an HTTP
/// client, issues a device token, and runs the sync. Every failure (client
/// build, token issuance, disk, HTTP) is logged at `warn` and swallowed: this
/// never returns an error and never blocks agent startup. Mirrors the
/// `reconcile_agent_version` startup step's log-and-continue handling.
pub async fn run_on_boot(layout: &Layout, backend_host: &BackendHost) {
    let client = match http::Client::new(&backend_host.as_url()) {
        Ok(client) => client,
        Err(e) => {
            warn!("metadata sync: failed to construct http client: {e}");
            return;
        }
    };

    let auth_dir = layout.auth();
    let private_key_file = auth_dir.private_key();
    let public_key_file = auth_dir.public_key();
    let token = match authn::issue_token(&client, &private_key_file, &public_key_file).await {
        Ok(token) => token,
        Err(e) => {
            warn!("metadata sync: failed to issue device token: {e}");
            return;
        }
    };

    match sync_system_metadata(&client, layout, &token.token).await {
        Ok(outcome) => debug!("metadata sync completed: {outcome:?}"),
        Err(e) => warn!("metadata sync failed: {e}"),
    }
}
