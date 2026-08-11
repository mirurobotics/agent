// standard crates
use std::future::Future;
use std::pin::Pin;

// internal crates
use crate::models::FileRuleUpload;
use crate::scan::{scanner::StableFile, ScanEvent, ScannerExt};
use crate::upload::{Job, UploaderExt};

// external crates
use tokio::sync::broadcast::error::RecvError;
use tracing::{debug, error, info, warn};

/// Bridge scanner observations into upload jobs. Subscribes to the scanner's
/// stable-file events and enqueues an upload job for each. Runs in the
/// background until shutdown.
pub async fn run<ScannerT: ScannerExt, UploaderT: UploaderExt>(
    scanner: &ScannerT,
    uploader: &UploaderT,
    mut shutdown_signal: Pin<Box<impl Future<Output = ()> + Send + 'static>>,
) {
    tokio::select! {
        _ = shutdown_signal.as_mut() => {
            info!("Scan-upload bridge worker shutdown complete");
        }
        // doesn't return but we do need to run it in the background
        _ = run_impl(scanner, uploader) => {}
    }
}

async fn run_impl<ScannerT: ScannerExt, UploaderT: UploaderExt>(
    scanner: &ScannerT,
    uploader: &UploaderT,
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
            Ok(ScanEvent::StableFile { file, upload }) => {
                enqueue_stable_file(uploader, file, upload).await;
            }
            // broadcast buffer overflowed.
            Err(RecvError::Lagged(dropped)) => {
                warn!(
                    "scan-upload bridge: lagged behind scanner events, dropped {dropped}; files will be re-observed on the next scan"
                );
            }
            // scanner is gone (all senders dropped). Log and idle so the only exit
            // is run()'s shutdown arm.
            Err(RecvError::Closed) => {
                error!(
                    "scan-upload bridge: scanner event stream ended; worker idling until shutdown"
                );
                return idle_forever().await;
            }
        }
    }
}

/// Enqueue an upload job for `stable`, if its rule uploads at all.
///
/// A rule with no `upload` block is retention-only: its files are scanned and
/// ledgered so the retention engine can act on them, but they never become
/// upload jobs.
async fn enqueue_stable_file<UploaderT: UploaderExt>(
    uploader: &UploaderT,
    stable: StableFile,
    upload: Option<FileRuleUpload>,
) {
    if upload.is_none() {
        let file = &stable.file;
        let rule_id = &stable.file_rule_id;
        debug!("scan-upload bridge: rule {rule_id} does not upload; skipping {file}");
        return;
    }

    let job = Job {
        file: stable.file,
        size: stable.size,
        digest: stable.digest,
        mtime: stable.mtime,
        first_observed_at: stable.first_observed_at,
        last_observed_at: stable.last_observed_at,
        file_rule_id: stable.file_rule_id,
        deployment_id: stable.deployment_id,
        retention: stable.retention,
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
