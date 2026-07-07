// standard crates
use std::future::Future;
use std::pin::Pin;

// internal crates
use crate::disk;
use crate::scan::{upload_rules::active_upload_configs, ScannerExt};
use crate::sync::{syncer::SyncEvent, SyncerExt};

// external crates
use tokio::sync::watch;
use tracing::{error, info};

/// Bridges the syncer to the scanner: it translates each successful sync into a
/// fresh push of the active upload rule set into the scanner. This keeps `sync`
/// and `scan` from depending on each other — the composition lives here in the
/// `workers` layer.
///
/// The local deployment cache reflects what was applied (regardless of whether
/// pushing deployments to the backend succeeded), so the rule set is recomputed
/// and pushed on every `SyncSuccess` to keep the scanner converged.
pub async fn run<SyncerT: SyncerExt, ScannerT: ScannerExt>(
    scanner: &ScannerT,
    deployments: &disk::Deployments,
    releases: &disk::Releases,
    upload_rules: &disk::UploadRules,
    syncer: &SyncerT,
    mut shutdown_signal: Pin<Box<impl Future<Output = ()> + Send + 'static>>,
) {
    tokio::select! {
        _ = shutdown_signal.as_mut() => {
            info!("Scan bridge worker shutdown complete");
        }
        // doesn't return but we do need to run it in the background
        _ = run_impl(scanner, deployments, releases, upload_rules, syncer) => {}
    }
}

async fn run_impl<SyncerT: SyncerExt, ScannerT: ScannerExt>(
    scanner: &ScannerT,
    deployments: &disk::Deployments,
    releases: &disk::Releases,
    upload_rules: &disk::UploadRules,
    syncer: &SyncerT,
) {
    info!("Running scan bridge worker");

    // subscribe to syncer events
    let mut syncer_subscriber = syncer.subscribe().await.unwrap_or_else(|e| {
        error!("error subscribing to syncer events: {e:?}");
        // Create a dummy receiver that never sends anything
        watch::channel(SyncEvent::SyncSuccess).1
    });

    // Seed the scanner once at startup. The watch channel's initial value is
    // `SyncSuccess`, but `changed()` alone will NOT observe that initial value,
    // so we must borrow-and-update explicitly here to deterministically push the
    // active rule set before entering the loop. Without this the scanner would
    // not receive any rules until the *next* successful sync.
    syncer_subscriber.borrow_and_update();
    push_active_configs(scanner, deployments, releases, upload_rules).await;

    while syncer_subscriber.changed().await.is_ok() {
        let syncer_event = syncer_subscriber.borrow().clone();
        if let SyncEvent::SyncSuccess = syncer_event {
            push_active_configs(scanner, deployments, releases, upload_rules).await;
        }
    }
}

/// Resolve the active upload config set from the local caches and push it into the
/// scanner. Errors are logged and swallowed so the bridge never crashes.
async fn push_active_configs<ScannerT: ScannerExt>(
    scanner: &ScannerT,
    deployments: &disk::Deployments,
    releases: &disk::Releases,
    upload_rules: &disk::UploadRules,
) {
    let cfgs = active_upload_configs(deployments, releases, upload_rules).await;
    if let Err(e) = scanner.set_configs(cfgs).await {
        error!("failed to push upload configs to scanner: {e:?}");
    }
}
