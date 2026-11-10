//! Pulling Miru's latest calibrations into a latest calibrations store, merging by `reported_at`.

// internal crates
use crate::calibrations::{
    errors::CalibrationsErr, latest::LatestStore, report::ReportMetadata, Filter,
};
use crate::http::{
    self,
    calibrations::{GetContentParams, ListAllLatestParams},
};

// external crates
use tracing::info;

/// Put Miru's latest calibration for each key into `store`, one key at a time,
/// unless the store's entry for that key was reported at the same time or
/// later, so reports Miru hasn't received yet are kept. A key the store
/// already has as new isn't downloaded, so a pull where nothing changed costs
/// one list call. Any failure fails the call; a new key past the store's
/// capacity fails it with `CalibrationStoreFullErr`, possibly after putting
/// some.
pub async fn pull_latest<HTTPClientT: http::ClientI>(
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
        put += usize::from(store.put_if_newer(metadata, &data).await?);
    }
    info!("calibrations: wrote {put} latest calibrations from Miru");
    Ok(())
}
