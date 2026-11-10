//! Merging Miru's latest calibrations, and reports still queued for Miru, into
//! a latest calibrations store.

// internal crates
use crate::calibrations::{
    errors::CalibrationsErr,
    latest::{LatestStore, DEFAULT_CAPACITY},
    report::{self, QueueSnapshot, ReportMetadata},
    Filter,
};
use crate::disk::layout::{CalibrationsLayout, LatestCalsLayout};
use crate::filesys::{files, FileSysErr};
use crate::http::{
    self,
    calibrations::{GetContentParams, ListAllLatestParams},
};

// external crates
use tracing::{info, warn};

/// Merge Miru's latest calibrations into the latest store in `layout`, as
/// [`merge_mirus_latest`] does. Any failure fails the call, so an emptied
/// store ends up holding every calibration Miru has; more than `capacity` keys
/// is `CalibrationStoreFullErr`.
pub async fn pull_latest<HTTPClientT: http::ClientI>(
    http_client: &HTTPClientT,
    layout: LatestCalsLayout,
    capacity: usize,
    token: &str,
) -> Result<(), CalibrationsErr> {
    // each command returns once done, so the worker exits idle when dropped
    let (handle, _) = LatestStore::spawn(64, layout, capacity);
    merge_mirus_latest(&handle, http_client, token).await
}

/// Mark the latest store in `layout` to merge in Miru's latest, for an agent
/// that didn't write the store until now.
pub async fn mark_needs_pull(layout: LatestCalsLayout) -> Result<(), CalibrationsErr> {
    let (handle, _) = LatestStore::spawn(64, layout, DEFAULT_CAPACITY);
    handle.set_needs_pull(true).await
}

/// Put each report queued in `layout` into its latest store, unless the
/// store's entry for that key was reported at the same time or later, for an
/// agent that queued reports without writing the store. A report whose data
/// can't be read is skipped.
pub async fn merge_queued(layout: &CalibrationsLayout) -> Result<(), CalibrationsErr> {
    let snapshot = match files::read_json::<QueueSnapshot>(&layout.queue()).await {
        Err(FileSysErr::PathDoesNotExistErr(_)) => return Ok(()),
        result => result?,
    };
    let (store, _) = LatestStore::spawn(64, layout.latest(), DEFAULT_CAPACITY);
    let reports = layout.reports();
    for entry in snapshot.entries {
        match report::read_data(&report::report_file(&reports, entry.id)).await {
            Ok(data) => {
                store.put_if_newer(entry.job, &data).await?;
            }
            Err(e) => warn!("calibrations: not merging queued report {}: {e}", entry.id),
        }
    }
    Ok(())
}

/// If `store` is marked, merge Miru's latest into it, then unmark it. A
/// failure leaves the mark, so the next call tries again.
pub async fn pull_if_needed<HTTPClientT: http::ClientI>(
    store: &LatestStore,
    http_client: &HTTPClientT,
    token: &str,
) -> Result<(), CalibrationsErr> {
    if !store.needs_pull().await? {
        return Ok(());
    }
    merge_mirus_latest(store, http_client, token).await?;
    store.set_needs_pull(false).await
}

/// Put Miru's latest calibration for each key, one key at a time, unless the
/// store's entry for that key was reported at the same time or later, so
/// reports Miru hasn't received yet are kept. A key the store already has as
/// new isn't downloaded, so a pull that failed part way resumes where it
/// stopped.
async fn merge_mirus_latest<HTTPClientT: http::ClientI>(
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
