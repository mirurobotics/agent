//! Pulling Miru's latest calibrations into a latest calibrations store.

// internal crates
use crate::calibrations::{errors::CalibrationsErr, latest::LatestStore, report::ReportMetadata};
use crate::disk::layout::LatestCalsLayout;
use crate::http::{
    self,
    calibrations::{GetContentParams, ListAllLatestParams},
};

// external crates
use tracing::info;

/// Write Miru's latest calibration for each key into the latest store in
/// `layout`, one key at a time, overwriting the entry already there for that
/// key. Any failure fails the call, so a store emptied first ends up holding
/// every calibration Miru has; more than `capacity` keys is
/// `CalibrationStoreFullErr`.
pub async fn pull_latest<HTTPClientT: http::ClientI>(
    http_client: &HTTPClientT,
    layout: LatestCalsLayout,
    capacity: usize,
    token: &str,
) -> Result<(), CalibrationsErr> {
    // each put returns once written, so the worker exits idle when dropped
    let (handle, _) = LatestStore::spawn(64, layout, capacity);
    let put = put_mirus_latest(&handle, http_client, token).await?;
    info!("calibrations: wrote {put} latest calibrations from Miru");
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
