// standard crates
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

// internal crates
use crate::disk;
use crate::scan::{errors::ScanErr, ScannerExt};
use crate::sync::{syncer::SyncEvent, SyncerExt};
use crate::workers::next_sync_event;

// external crates
use tracing::{error, info};

pub struct Storage {
    pub deployments: Arc<disk::Deployments>,
    pub releases: Arc<disk::Releases>,
    pub upload_rules: Arc<disk::UploadRules>,
}

pub async fn run<ScannerT: ScannerExt, SyncerT: SyncerExt>(
    scanner: &ScannerT,
    syncer: &SyncerT,
    storage: &Storage,
    mut shutdown_signal: Pin<Box<impl Future<Output = ()> + Send + 'static>>,
) {
    tokio::select! {
        _ = shutdown_signal.as_mut() => {
            info!("Scan bridge worker shutdown complete");
        }
        // doesn't return but we do need to run it in the background
        _ = run_impl(scanner, syncer, storage) => {}
    }
}

async fn run_impl<ScannerT: ScannerExt, SyncerT: SyncerExt>(
    scanner: &ScannerT,
    syncer: &SyncerT,
    storage: &Storage,
) {
    info!("Running scan bridge worker");

    let mut subscriber = match syncer.subscribe().await {
        Ok(subscriber) => subscriber,
        Err(e) => {
            error!("scan bridge: error subscribing to syncer events: {e:?}; idling until shutdown");
            return idle_forever().await;
        }
    };

    // consume the seed value FIRST so a SyncSuccess landing during the startup disk
    // read is not coalesced away by borrow_and_update after the read.
    let _ = subscriber.borrow_and_update();

    // resolve and push once at startup so the scanner reflects current deployment
    resolve_and_push(scanner, storage).await;

    let mut subscriber = Some(subscriber);
    while let Some(event) = next_sync_event(&mut subscriber).await {
        if matches!(event, SyncEvent::SyncSuccess) {
            resolve_and_push(scanner, storage).await;
        }
    }

    // sync stream ended (all Senders dropped). Do NOT return — that would silently
    // retire the worker while the app believes it is live. Log and idle so the only
    // exit is run()'s shutdown arm.
    error!("scan bridge: syncer event stream ended; worker idling until shutdown");
    idle_forever().await
}

/// Park forever. Used when the worker has nothing left to react to but must not return
/// — run()'s shutdown arm is the only intended exit.
async fn idle_forever() {
    std::future::pending::<()>().await
}

/// Resolve active upload rules from disk and push them into the scanner
async fn resolve_and_push<ScannerT: ScannerExt>(scanner: &ScannerT, storage: &Storage) {
    let result = disk::upload_rules_for_deployed(
        &storage.deployments,
        &storage.releases,
        &storage.upload_rules,
    )
    .await
    .map_err(disk_err_to_scan_err);
    match result {
        Ok(Some((deployment, rules))) => {
            if let Err(e) = scanner.update_rules(deployment, rules).await {
                error!("scan bridge: failed to update scanner rules: {e:?}");
            }
        }
        Ok(None) => {
            if let Err(e) = scanner.clear_rules().await {
                error!("scan bridge: failed to clear scanner rules: {e:?}");
            }
        }
        Err(e) => {
            error!("scan bridge: failed to resolve active upload rules: {e:?}");
        }
    }
}

fn disk_err_to_scan_err(e: disk::DiskErr) -> ScanErr {
    match e {
        disk::DiskErr::CacheErr(c) => ScanErr::CacheErr(c),
        disk::DiskErr::FileSysErr(f) => ScanErr::FileSysErr(f),
        other => ScanErr::InternalError(crate::scan::errors::InternalError {
            message: format!("unexpected disk error resolving upload rules: {other:?}"),
            trace: crate::trace!(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace;

    // A DiskErr::CacheErr maps onto ScanErr::CacheErr.
    #[test]
    fn disk_cache_err_maps_to_scan_cache_err() {
        let err = disk::DiskErr::CacheErr(crate::cache::CacheErr::CacheElementNotFound(
            crate::cache::errors::CacheElementNotFound {
                msg: "missing".to_string(),
                trace: trace!(),
            },
        ));
        assert!(matches!(disk_err_to_scan_err(err), ScanErr::CacheErr(_)));
    }

    // A DiskErr::FileSysErr maps onto ScanErr::FileSysErr.
    #[test]
    fn disk_filesys_err_maps_to_scan_filesys_err() {
        let err = disk::DiskErr::FileSysErr(crate::filesys::FileSysErr::PathDoesNotExistErr(
            crate::filesys::errors::PathDoesNotExistErr {
                path: std::path::PathBuf::from("/nope"),
                trace: trace!(),
            },
        ));
        assert!(matches!(disk_err_to_scan_err(err), ScanErr::FileSysErr(_)));
    }

    // Any other DiskErr variant surfaces as an internal error (unexpected here).
    #[test]
    fn other_disk_err_maps_to_internal_error() {
        let err = disk::DiskErr::DeviceNotActivatedErr(crate::disk::DeviceNotActivatedErr {
            msg: "not activated".to_string(),
            trace: trace!(),
        });
        assert!(matches!(
            disk_err_to_scan_err(err),
            ScanErr::InternalError(_)
        ));
    }
}
