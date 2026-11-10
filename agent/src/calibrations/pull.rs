//! Pulling the control plane's latest calibrations into a latest calibrations
//! store, merging by `reported_at`.

// standard crates
use std::time::Duration;

// internal crates
use crate::authn::TokenManagerExt;
use crate::calibrations::{
    errors::CalibrationsErr, latest::LatestStore, report::ReportMetadata, Filter,
};
use crate::cooldown;
use crate::http::{
    self,
    calibrations::{GetContentParams, ListAllLatestParams},
    HTTPErr,
};

// external crates
use backend_api::models::Calibration;
use tracing::{info, warn};

const PULL_BACKOFF: cooldown::Backoff = cooldown::Backoff {
    base_secs: 1,
    growth_factor: 2,
    max_secs: 60,
};

/// Put the control plane's latest calibration for each key into `store`,
/// unless the store's entry is as new or newer. Skipped keys aren't downloaded.
pub async fn pull_latest<HTTPClientT: http::ClientI>(
    store: &LatestStore,
    http_client: &HTTPClientT,
    token: &str,
) -> Result<(), CalibrationsErr> {
    let calibrations = list_latest(http_client, token).await?;
    let stored = store.list(&Filter::default()).await?;
    let mut put = 0;
    for calibration in calibrations {
        put += usize::from(pull_one(store, &stored, http_client, token, calibration).await?);
    }
    info!("calibrations: wrote {put} latest calibrations from the control plane");
    Ok(())
}

async fn list_latest<HTTPClientT: http::ClientI>(
    http_client: &HTTPClientT,
    token: &str,
) -> Result<Vec<Calibration>, HTTPErr> {
    http::with_retry(|| {
        http::calibrations::list_all_latest(
            http_client,
            ListAllLatestParams {
                expansions: &["reported_by"],
                token,
            },
        )
    })
    .await
}

/// Put `calibration` into `store` unless `stored` has its key as new or newer.
/// Returns whether it was put.
async fn pull_one<HTTPClientT: http::ClientI>(
    store: &LatestStore,
    stored: &[ReportMetadata],
    http_client: &HTTPClientT,
    token: &str,
    calibration: Calibration,
) -> Result<bool, CalibrationsErr> {
    let id = calibration.id.clone();
    let metadata = ReportMetadata::from(calibration);
    // `stored` predates the downloads, so this check only skips downloads.
    // put_if_newer checks again as it writes, in case intake put a newer
    // report for the key in the meantime.
    if stored.iter().any(|entry| entry.is_as_new_as(&metadata)) {
        return Ok(false);
    }
    let data = download(http_client, &id, token).await?;
    store.put_if_newer(metadata, &data).await
}

async fn download<HTTPClientT: http::ClientI>(
    http_client: &HTTPClientT,
    id: &str,
    token: &str,
) -> Result<String, HTTPErr> {
    http::with_retry(|| {
        http::calibrations::get_content(http_client, GetContentParams { id, token })
    })
    .await
}

/// Pull into `store` until a pull succeeds, waiting longer after each failure.
pub async fn pull_latest_until_success<HTTPClientT, TokenManagerT>(
    store: &LatestStore,
    http_client: &HTTPClientT,
    token_mngr: &TokenManagerT,
) where
    HTTPClientT: http::ClientI,
    TokenManagerT: TokenManagerExt,
{
    let mut attempts: u32 = 0;
    loop {
        let result = match token_mngr.get_token().await {
            Ok(token) => pull_latest(store, http_client, &token.token).await,
            Err(e) => Err(e.into()),
        };
        let e = match result {
            Ok(()) => return,
            Err(e) => e,
        };
        attempts = attempts.saturating_add(1);
        let wait = cooldown::calc(&PULL_BACKOFF, attempts);
        warn!("calibrations: unable to pull the latest calibrations, retrying in {wait}s: {e}");
        tokio::time::sleep(Duration::from_secs(wait as u64)).await;
    }
}
