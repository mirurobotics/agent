//! Loading a latest calibrations store from Miru.

// internal crates
use crate::calibrations::{errors::CalibrationsErr, latest::LatestStore, report::ReportMetadata};
use crate::disk::layout::LatestCalsLayout;
use crate::errors::Error;
use crate::http::{
    self,
    calibrations::{GetContentParams, ListAllLatestParams},
};

// external crates
use tracing::{error, info, warn};

/// Fill the empty latest store in `store` with Miru's latest calibration for
/// each key. Writes one key at a time and holds at most `capacity` keys,
/// skipping Miru's extra keys with a warning.
pub async fn load_latest<HTTPClientT: http::ClientI>(
    http_client: &HTTPClientT,
    store: LatestCalsLayout,
    capacity: usize,
    token: &str,
) -> Result<(), CalibrationsErr> {
    let (handle, worker) = LatestStore::spawn(64, store, capacity);
    let result = put_mirus_latest(&handle, http_client, token, capacity).await;
    // awaiting the worker leaves no task writing the store after the load
    drop(handle);
    if let Err(e) = worker.await {
        error!("calibrations: the latest store's worker failed: {e}");
    }
    let put = result?;
    info!("calibrations: loaded {put} latest calibrations from Miru");
    Ok(())
}

/// Put Miru's latest calibration for each key, up to `capacity`. Returns the
/// number put.
async fn put_mirus_latest<HTTPClientT: http::ClientI>(
    store: &LatestStore,
    http_client: &HTTPClientT,
    token: &str,
    capacity: usize,
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

    let mut put: usize = 0;
    let mut skipped: usize = 0;
    for calibration in calibrations {
        if put >= capacity {
            skipped += 1;
            continue;
        }
        let id = calibration.id.clone();
        let Some(data) = download(http_client, token, &id).await? else {
            continue;
        };
        store.put(ReportMetadata::from(calibration), &data).await?;
        put += 1;
    }

    if skipped > 0 {
        warn!("calibrations: the latest store holds {capacity} keys; skipped {skipped} of Miru's latest calibrations");
    }
    Ok(put)
}

/// The content of Miru's calibration `id`, or `None` when it fails for a
/// reason other than the network, so a calibration Miru can't serve doesn't
/// block every load.
async fn download<HTTPClientT: http::ClientI>(
    http_client: &HTTPClientT,
    token: &str,
    id: &str,
) -> Result<Option<String>, CalibrationsErr> {
    let result = http::with_retry(|| {
        http::calibrations::get_content(http_client, GetContentParams { id, token })
    })
    .await;
    match result {
        Ok(data) => Ok(Some(data)),
        Err(e) if e.is_network_conn_err() => Err(e.into()),
        Err(e) => {
            error!("calibrations: skipping Miru's calibration {id}, whose content failed to download: {e}");
            Ok(None)
        }
    }
}
