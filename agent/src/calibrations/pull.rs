//! Pulling the control plane's latest calibrations into a latest calibrations
//! store, merging by `reported_at`.

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

/// Put the control plane's latest calibration for each key into `store`,
/// unless the store's entry is as new or newer. Skipped keys aren't downloaded.
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
        // only saves the download: put_if_newer checks again as it writes,
        // since intake may put a newer report for the key meanwhile
        if stored.iter().any(|entry| entry.is_as_new_as(&metadata)) {
            continue;
        }
        let data = http::with_retry(|| {
            http::calibrations::get_content(http_client, GetContentParams { id: &id, token })
        })
        .await?;
        put += usize::from(store.put_if_newer(metadata, &data).await?);
    }
    info!("calibrations: wrote {put} latest calibrations from the control plane");
    Ok(())
}
