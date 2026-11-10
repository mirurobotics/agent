//! Pulling Miru's latest calibrations into a latest calibrations store.

// standard crates
use std::collections::HashMap;

// internal crates
use crate::calibrations::{
    errors::CalibrationsErr,
    latest::LatestStore,
    report::{Key, ReportMetadata},
    Filter,
};
use crate::disk::layout::LatestCalsLayout;
use crate::http::{
    self,
    calibrations::{GetContentParams, ListAllLatestParams},
};

// external crates
use chrono::{DateTime, Utc};
use tracing::info;

/// Write Miru's latest calibration for each key into the latest store in
/// `layout`, one key at a time, unless the store's entry for that key is as new
/// or newer, so reports Miru hasn't received yet are kept. Any failure fails
/// the call, so an emptied store ends up holding every calibration Miru has;
/// more than `capacity` keys is `CalibrationStoreFullErr`. The caller must keep
/// the agent from writing the store meanwhile.
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

/// Put Miru's latest calibration for each key the store has nothing as new
/// for. Returns the number put.
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
    let entries = store.list(&Filter::default()).await?;
    let stored: HashMap<Key<'_>, DateTime<Utc>> =
        entries.iter().map(|m| (m.key(), m.reported_at)).collect();

    let mut put = 0;
    for calibration in calibrations {
        let id = calibration.id.clone();
        let metadata = ReportMetadata::from(calibration);
        if !is_newer(&metadata, &stored) {
            continue;
        }
        let data = http::with_retry(|| {
            http::calibrations::get_content(http_client, GetContentParams { id: &id, token })
        })
        .await?;
        store.put(metadata, &data).await?;
        put += 1;
    }
    Ok(put)
}

/// Whether `metadata` was reported after the stored entry for its key, or the
/// store has none.
fn is_newer(metadata: &ReportMetadata, stored: &HashMap<Key<'_>, DateTime<Utc>>) -> bool {
    stored
        .get(&metadata.key())
        .is_none_or(|reported_at| metadata.reported_at > *reported_at)
}
