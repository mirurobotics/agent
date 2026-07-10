// standard crates
use std::future::Future;
use std::pin::Pin;

// internal crates
use crate::disk;
use crate::models::{Deployment, UploadRule};
use crate::scan::{errors::ScanErr, ScannerExt};
use crate::sync::{syncer::SyncEvent, SyncerExt};

// external crates
use tokio::sync::watch;
use tracing::{error, info};

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

    // subscribe to syncer events
    let mut subscriber = syncer.subscribe().await.unwrap_or_else(|e| {
        error!("error subscribing to syncer events: {e:?}");
        // Create a dummy receiver that never sends anything
        watch::channel(SyncEvent::SyncSuccess).1
    });

    // resolve and push once at startup so the scanner reflects the deployed rules
    // even if no sync happens after this worker starts.
    resolve_and_push(scanner, deployments, releases, upload_rules).await;
    // consume the seed value so we only react to subsequent changes.
    let _ = subscriber.borrow_and_update();

    while subscriber.changed().await.is_ok() {
        if matches!(&*subscriber.borrow_and_update(), SyncEvent::SyncSuccess) {
            resolve_and_push(scanner, deployments, releases, upload_rules).await;
        }
    }
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
/// rules from the disk stores.
async fn resolve_active_rules(
    deployments: &disk::Deployments,
    releases: &disk::Releases,
    upload_rules: &disk::UploadRules,
) -> Result<Option<(Deployment, Vec<UploadRule>)>, ScanErr> {
    match crate::disk::deployments::find_deployed(deployments).await? {
        Some(deployment) => {
            let rules =
                disk::upload_rules_for_deployment(releases, upload_rules, &deployment).await?;
            Ok(Some((deployment, rules)))
        }
        None => Ok(None),
    }
}
