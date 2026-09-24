// internal crates
use crate::disk;
use crate::filesys::File;
use crate::http;
use crate::models;
use crate::sync::errors::SyncErr;

// external crates
use tracing::{debug, warn};

pub struct SyncArgs<'a, HTTPClientT> {
    pub http_client: &'a HTTPClientT,
    pub device: &'a disk::Device,
    pub cache_file: &'a File,
    pub token: &'a str,
}

/// Outcome of a successful [`sync`] run.
#[derive(Debug, PartialEq, Eq)]
pub enum Synced {
    /// Live metadata matched the cache; no network call was performed.
    Unchanged,
    /// Live metadata differed from the cache (or the cache was absent); the
    /// backend device was updated and the cache refreshed.
    Updated,
}

/// Compare the running host's system metadata against the on-disk cache of what
/// was last reported and, only when they differ (or the cache is absent or
/// unreadable), update the backend device with the metadata fields and refresh
/// the cache.
///
/// The cache is written **only after** a confirmed successful update, so a
/// failed update returns `Err` before the cache is touched and is retried on
/// the next sync. A missing or unreadable cache is treated as "no cache" (the
/// read error is logged, never propagated).
pub async fn sync<HTTPClientT: http::ClientI>(
    args: &SyncArgs<'_, HTTPClientT>,
) -> Result<Synced, SyncErr> {
    let live = models::system_metadata();

    let cached = match disk::system_metadata::read(args.cache_file).await {
        Ok(cached) => cached,
        Err(e) => {
            warn!("unable to read system metadata cache; treating as absent: {e}");
            None
        }
    };
    if cached.as_ref() == Some(&live) {
        debug!("system metadata unchanged since last report; skipping device update");
        return Ok(Synced::Unchanged);
    }

    let device = args.device.read().await?;
    http::devices::update(
        args.http_client,
        http::devices::UpdateParams {
            id: &device.id,
            payload: &live.to_update_request(),
            token: args.token,
        },
    )
    .await?;

    disk::system_metadata::write(args.cache_file, &live).await?;
    Ok(Synced::Updated)
}
