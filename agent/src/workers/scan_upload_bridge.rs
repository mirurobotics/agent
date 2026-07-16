// standard crates
use std::future::Future;
use std::pin::Pin;

// internal crates
use crate::disk::Deployments;
use crate::scan::{scanner::StableFile, ScanEvent, ScannerExt};
use crate::upload::{Job, UploaderExt};

// external crates
use tokio::sync::broadcast::error::RecvError;
use tracing::{error, info, warn};

/// Bridge scanner observations into upload jobs. Subscribes to the scanner's
/// stable-file events, resolves each file's release from disk, and enqueues an
/// upload job. Runs in the background until shutdown.
pub async fn run<ScannerT: ScannerExt, UploaderT: UploaderExt>(
    scanner: &ScannerT,
    uploader: &UploaderT,
    deployments: &Deployments,
    mut shutdown_signal: Pin<Box<impl Future<Output = ()> + Send + 'static>>,
) {
    tokio::select! {
        _ = shutdown_signal.as_mut() => {
            info!("Scan-upload bridge worker shutdown complete");
        }
        // doesn't return but we do need to run it in the background
        _ = run_impl(scanner, uploader, deployments) => {}
    }
}

async fn run_impl<ScannerT: ScannerExt, UploaderT: UploaderExt>(
    scanner: &ScannerT,
    uploader: &UploaderT,
    deployments: &Deployments,
) {
    info!("Running scan-upload bridge worker");

    let mut events = match scanner.subscribe().await {
        Ok(events) => events,
        Err(e) => {
            error!(
                "scan-upload bridge: error subscribing to scanner events: {e:?}; idling until shutdown"
            );
            return idle_forever().await;
        }
    };

    loop {
        match events.recv().await {
            Ok(ScanEvent::StableFile(stable)) => {
                enqueue_stable_file(uploader, deployments, stable).await;
            }
            // the broadcast buffer overflowed. The dropped files are re-emitted on the
            // scanner's next pass (backend digest dedup absorbs any overlap), so log and
            // keep consuming rather than treating this as fatal.
            Err(RecvError::Lagged(dropped)) => {
                warn!(
                    "scan-upload bridge: lagged behind scanner events, dropped {dropped}; files will be re-observed on the next scan"
                );
            }
            // the scanner is gone (all senders dropped). Do NOT return — that would
            // silently retire the worker while the app believes it is live. Log and idle
            // so the only exit is run()'s shutdown arm.
            Err(RecvError::Closed) => {
                error!(
                    "scan-upload bridge: scanner event stream ended; worker idling until shutdown"
                );
                return idle_forever().await;
            }
        }
    }
}

/// Resolve `stable`'s release from its deployment and enqueue an upload job.
/// A file whose deployment cannot be resolved is skipped (logged, not fatal):
/// it is re-observed on a later scan once the deployment is readable again.
async fn enqueue_stable_file<UploaderT: UploaderExt>(
    uploader: &UploaderT,
    deployments: &Deployments,
    stable: StableFile,
) {
    // the scanner records the deployment a file was observed under but not its
    // release; resolve the release from the deployment record on disk.
    let deployment = match deployments.read(stable.deployment_id.clone()).await {
        Ok(deployment) => deployment,
        Err(e) => {
            warn!(
                "scan-upload bridge: cannot resolve deployment {} for file {} (rule {}): {e:?}; skipping",
                stable.deployment_id, stable.file, stable.upload_rule_id
            );
            return;
        }
    };

    let job = Job {
        file: stable.file,
        size: stable.size,
        digest: stable.digest,
        mtime: stable.mtime,
        first_observed_at: stable.first_observed_at,
        last_observed_at: stable.last_observed_at,
        upload_rule_id: stable.upload_rule_id,
        deployment_id: stable.deployment_id,
        release_id: deployment.release_id,
    };

    if let Err(e) = uploader.enqueue(job).await {
        warn!("scan-upload bridge: failed to enqueue upload job: {e:?}");
    }
}

/// Park forever. Used when the worker has nothing left to react to but must not
/// return — run()'s shutdown arm is the only intended exit.
async fn idle_forever() {
    std::future::pending::<()>().await
}
