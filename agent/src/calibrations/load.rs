//! Loading a latest calibrations store from the upload queue and Miru.

// standard crates
use std::collections::{HashMap, HashSet};

// internal crates
use crate::calibrations::{
    errors::CalibrationsErr,
    latest::LatestStore,
    report::{self, Key, QueueEntry, QueueSnapshot, ReportMetadata},
};
use crate::disk::layout::{CalibrationsLayout, LatestCalsLayout};
use crate::errors::Error;
use crate::filesys::{files, Dir, FileSysErr};
use crate::http::{
    self,
    calibrations::{GetContentParams, ListAllLatestParams},
};

// external crates
use tracing::{error, info, warn};

/// Fill the empty latest store in `store` with the newest report per key
/// still in `queue`, then Miru's latest calibration for every other key.
/// Writes one key at a time and holds at most `capacity` keys, skipping
/// Miru's extra keys with a warning.
pub async fn load_latest<HTTPClientT: http::ClientI>(
    http_client: &HTTPClientT,
    queue: &CalibrationsLayout,
    store: LatestCalsLayout,
    capacity: usize,
    token: &str,
) -> Result<(), CalibrationsErr> {
    let entries = queued_entries(queue).await;
    let queued = newest_per_key(&entries);
    let (handle, worker) = LatestStore::spawn(64, store, capacity);
    let reports = queue.reports();
    let result = fill(&handle, http_client, token, queued, &reports, capacity).await;
    // awaiting the worker leaves no task writing the store after the load
    drop(handle);
    if let Err(e) = worker.await {
        error!("calibrations: the latest store's worker failed: {e}");
    }
    result
}

/// The reports in the upload queue. Only reads the queue file, which the
/// uploader owns; a missing file is empty, and one that can't be read is
/// logged and read as empty.
async fn queued_entries(layout: &CalibrationsLayout) -> Vec<QueueEntry> {
    match files::read_json::<QueueSnapshot>(&layout.queue()).await {
        Ok(snapshot) => snapshot.entries,
        Err(FileSysErr::PathDoesNotExistErr(_)) => Vec::new(),
        Err(e) => {
            error!("calibrations: loading from Miru only, the upload queue can't be read: {e}");
            Vec::new()
        }
    }
}

/// The newest of `entries` for each key, by `reported_at`, with the later
/// entry winning a tie. Retries move a report to the tail, so queue order
/// isn't report order.
fn newest_per_key(entries: &[QueueEntry]) -> Vec<&QueueEntry> {
    let mut newest: HashMap<Key<'_>, &QueueEntry> = HashMap::new();
    for entry in entries {
        let kept = newest.entry(entry.job.key()).or_insert(entry);
        if entry.job.reported_at >= kept.job.reported_at {
            *kept = entry;
        }
    }
    newest.into_values().collect()
}

async fn fill<HTTPClientT: http::ClientI>(
    store: &LatestStore,
    http_client: &HTTPClientT,
    token: &str,
    queued: Vec<&QueueEntry>,
    reports: &Dir,
    capacity: usize,
) -> Result<(), CalibrationsErr> {
    let queued_keys = put_queued(store, reports, queued).await?;
    let from_miru = put_mirus_latest(store, http_client, token, &queued_keys, capacity).await?;
    let from_queue = queued_keys.len();
    info!("calibrations: loaded {from_miru} latest calibrations from Miru and {from_queue} from the upload queue");
    Ok(())
}

/// Put each of `queued`'s reports, skipping one whose file is missing or not
/// UTF-8, which the uploader drops too. Returns the keys put.
async fn put_queued<'a>(
    store: &LatestStore,
    reports: &Dir,
    queued: Vec<&'a QueueEntry>,
) -> Result<HashSet<Key<'a>>, CalibrationsErr> {
    let mut put = HashSet::new();
    for entry in queued {
        let file = report::report_file(reports, entry.id);
        let data = match report::read_data(&file).await {
            Ok(data) => data,
            Err(CalibrationsErr::InvalidReportErr(e)) => {
                warn!("calibrations: skipping the queued report in {file}: {e}");
                continue;
            }
            Err(e) => return Err(e),
        };
        store.put(entry.job.clone(), &data).await?;
        put.insert(entry.job.key());
    }
    Ok(put)
}

/// Put Miru's latest calibration for each key not in `queued_keys`, in the
/// room the queued reports leave. Returns the number put.
async fn put_mirus_latest<HTTPClientT: http::ClientI>(
    store: &LatestStore,
    http_client: &HTTPClientT,
    token: &str,
    queued_keys: &HashSet<Key<'_>>,
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

    let room = capacity.saturating_sub(queued_keys.len());
    let mut put: usize = 0;
    let mut skipped: usize = 0;
    for calibration in calibrations {
        let id = calibration.id.clone();
        let metadata = ReportMetadata::from(calibration);
        if queued_keys.contains(&metadata.key()) {
            continue;
        }
        if put >= room {
            skipped += 1;
            continue;
        }
        let Some(data) = download(http_client, token, &id).await? else {
            continue;
        };
        store.put(metadata, &data).await?;
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
