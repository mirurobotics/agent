//! Pulling the control plane's latest calibrations into a latest calibrations
//! store, merging by `reported_at`.

// internal crates
use crate::calibrations::{
    errors::CalibrationsErr, latest::LatestStore, report::ReportMetadata, Filter,
};
use crate::http::{
    self,
    calibrations::{GetContentParams, ListAllLatestParams},
    HTTPErr,
};

// external crates
use backend_api::models::Calibration;
use tracing::info;

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
