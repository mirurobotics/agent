//! Loading a latest calibrations store from Miru.

// internal crates
use crate::calibrations::{errors::CalibrationsErr, latest::LatestStore, report::ReportMetadata};
use crate::disk::layout::LatestCalsLayout;
use crate::http::{
    self,
    calibrations::{GetContentParams, ListAllLatestParams},
};

// external crates
use tracing::{error, info};

/// Fill the empty latest store in `store` with Miru's latest calibration for
/// each key, one key at a time. Any failure fails the load, so a loaded store
/// holds every calibration Miru has; more than `capacity` keys is
/// `CalibrationStoreFullErr`.
pub async fn load_latest<HTTPClientT: http::ClientI>(
    http_client: &HTTPClientT,
    store: LatestCalsLayout,
    capacity: usize,
    token: &str,
) -> Result<(), CalibrationsErr> {
    let (handle, worker) = LatestStore::spawn(64, store, capacity);
    let result = put_mirus_latest(&handle, http_client, token).await;
    // awaiting the worker leaves no task writing the store after the load
    drop(handle);
    if let Err(e) = worker.await {
        error!("calibrations: the latest store's worker failed: {e}");
    }
    let put = result?;
    info!("calibrations: loaded {put} latest calibrations from Miru");
    Ok(())
}

/// Put Miru's latest calibration for each key. Returns the number put.
async fn put_mirus_latest<HTTPClientT: http::ClientI>(
    store: &LatestStore,
    http_client: &HTTPClientT,
    token: &str,
) -> Result<usize, CalibrationsErr> {
    let calibrations = http::with_retry(|| {
        http::calibrations::list_all_latest(
            http_client,
            ListAllLatestParams {
                expansions: &["reported_by"],
                token,
            },
        )
    })
    .await?;

    let put = calibrations.len();
    for calibration in calibrations {
        let id = calibration.id.as_str();
        let data = http::with_retry(|| {
            http::calibrations::get_content(http_client, GetContentParams { id, token })
        })
        .await?;
        store.put(ReportMetadata::from(calibration), &data).await?;
    }
    Ok(put)
}
