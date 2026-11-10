//! Merging Miru's latest calibrations into a latest calibrations store.

// internal crates
use crate::calibrations::{
    errors::CalibrationsErr, latest::LatestStore, report::ReportMetadata, Filter,
};
use crate::disk::layout::LatestCalsLayout;
use crate::http::{
    self,
    calibrations::{GetContentParams, ListAllLatestParams},
};

// external crates
use tracing::info;

/// Merge Miru's latest calibrations into the latest store in `layout`, as
/// [`merge_latest`] does. Any failure fails the call, so an emptied store ends
/// up holding every calibration Miru has; more than `capacity` keys is
/// `CalibrationStoreFullErr`.
pub async fn pull_latest<HTTPClientT: http::ClientI>(
    http_client: &HTTPClientT,
    layout: LatestCalsLayout,
    capacity: usize,
    token: &str,
) -> Result<(), CalibrationsErr> {
    // each command returns once done, so the worker exits idle when dropped
    let (handle, _) = LatestStore::spawn(64, layout, capacity);
    merge_latest(&handle, http_client, token).await
}

/// Put Miru's latest calibration for each key into `store`, one key at a time,
/// unless the store's entry for that key was reported at the same time or
/// later, so reports Miru hasn't received yet are kept. A key the store
/// already has as new isn't downloaded, so a merge where nothing changed costs
/// one list call.
pub async fn merge_latest<HTTPClientT: http::ClientI>(
    store: &LatestStore,
    http_client: &HTTPClientT,
    token: &str,
) -> Result<(), CalibrationsErr> {
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
    let stored = store.list(&Filter::default()).await?;

    let mut put = 0;
    for calibration in calibrations {
        let id = calibration.id.clone();
        let metadata = ReportMetadata::from(calibration);
        if stored.iter().any(|entry| entry.is_as_new_as(&metadata)) {
            continue;
        }
        let data = http::with_retry(|| {
            http::calibrations::get_content(http_client, GetContentParams { id: &id, token })
        })
        .await?;
        if store.put_if_newer(metadata, &data).await? {
            put += 1;
        }
    }
    info!("calibrations: wrote {put} latest calibrations from Miru");
    Ok(())
}
