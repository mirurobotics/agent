// standard crates
use std::future::Future;
use std::pin::Pin;

// internal crates
use crate::disk;
use crate::models::{Deployment, UploadRule};
use crate::scan::{errors::ScanErr, ScannerExt};
use crate::sync::{syncer::SyncEvent, SyncerExt};

// external crates
use tracing::{error, info};

/// Park forever. Used when the worker has nothing left to react to but must not
/// return — run()'s shutdown arm is the only intended exit.
async fn idle_forever() {
    std::future::pending::<()>().await
}

pub async fn run<ScannerT: ScannerExt, SyncerT: SyncerExt>(
    scanner: &ScannerT,
    syncer: &SyncerT,
    deployments: &disk::Deployments,
    releases: &disk::Releases,
    upload_rules: &disk::UploadRules,
    mut shutdown_signal: Pin<Box<impl Future<Output = ()> + Send + 'static>>,
) {
    tokio::select! {
        _ = shutdown_signal.as_mut() => {
            info!("Scan bridge worker shutdown complete");
        }
        // doesn't return but we do need to run it in the background
        _ = run_impl(
            scanner,
            syncer,
            deployments,
            releases,
            upload_rules,
        ) => {}
    }
}

async fn run_impl<ScannerT: ScannerExt, SyncerT: SyncerExt>(
    scanner: &ScannerT,
    syncer: &SyncerT,
    deployments: &disk::Deployments,
    releases: &disk::Releases,
    upload_rules: &disk::UploadRules,
) {
    info!("Running scan bridge worker");

    // subscribe to syncer events; on a transient subscribe failure, idle rather
    // than fall through to a dead loop (a dummy receiver's Sender is already
    // dropped, so changed() would Err on the first poll and disable the worker).
    let mut subscriber = if let Ok(rx) = syncer.subscribe().await {
        rx
    } else {
        error!("scan bridge: error subscribing to syncer events; idling until shutdown");
        return idle_forever().await;
    };

    // Consume the seed value FIRST so a SyncSuccess landing during the startup
    // disk read is not coalesced away by borrow_and_update after the read.
    let _ = subscriber.borrow_and_update();

    // Resolve and push once at startup so the scanner reflects the deployed
    // rules even if no sync happens after this worker starts. Idempotent, so a
    // redundant re-resolve on the next event is harmless.
    resolve_and_push(scanner, deployments, releases, upload_rules).await;

    while subscriber.changed().await.is_ok() {
        if matches!(&*subscriber.borrow_and_update(), SyncEvent::SyncSuccess) {
            resolve_and_push(scanner, deployments, releases, upload_rules).await;
        }
    }

    // The sync stream ended (all Senders dropped). Do NOT return — that would
    // silently retire the worker while the app believes it is live. Log and idle
    // so the only exit is run()'s shutdown arm.
    error!("scan bridge: syncer event stream ended; worker idling until shutdown");
    idle_forever().await
}

/// Resolve the active upload rules from disk and push them into the scanner,
/// logging (never propagating) any error so the worker survives to handle the
/// next event.
async fn resolve_and_push<ScannerT: ScannerExt>(
    scanner: &ScannerT,
    deployments: &disk::Deployments,
    releases: &disk::Releases,
    upload_rules: &disk::UploadRules,
) {
    match resolve_active_rules(deployments, releases, upload_rules).await {
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

/// Resolve the currently Deployed deployment (if any) and its active upload
/// rules from the disk stores, mapping the disk error space onto the scanner's.
async fn resolve_active_rules(
    deployments: &disk::Deployments,
    releases: &disk::Releases,
    upload_rules: &disk::UploadRules,
) -> Result<Option<(Deployment, Vec<UploadRule>)>, ScanErr> {
    disk::upload_rules_for_deployed(deployments, releases, upload_rules)
        .await
        .map_err(disk_err_to_scan_err)
}

/// Map the disk errors reachable from the deployed-rules query onto the
/// scanner's own error space. Only cache/filesystem reads occur here; any other
/// DiskErr variant is unexpected and surfaces as an internal error.
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
