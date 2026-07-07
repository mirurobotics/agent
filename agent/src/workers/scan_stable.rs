// standard crates
use std::future::Future;
use std::pin::Pin;

// internal crates
use crate::filesys::PathExt;
use crate::scan::{scanner::ScanEvent, ScannerExt};

// external crates
use tokio::sync::broadcast::error::RecvError;
use tracing::{info, warn};

/// Subscribes to newly-stable file events from the scanner and dispatches them
/// to consumers. The M2 placeholder consumer logs each file; M3 replaces that
/// branch with the digest + POST /uploads pipeline (uploader).
pub async fn run<ScannerT: ScannerExt>(
    scanner: &ScannerT,
    mut shutdown_signal: Pin<Box<impl Future<Output = ()> + Send + 'static>>,
) {
    tokio::select! {
        _ = shutdown_signal.as_mut() => {
            info!("Scan stable worker shutdown complete");
        }
        _ = run_impl(scanner) => {}
    }
}

async fn run_impl<ScannerT: ScannerExt>(scanner: &ScannerT) {
    info!("Running scan stable worker");

    let mut subscriber = match scanner.subscribe().await {
        Ok(subscriber) => subscriber,
        Err(e) => {
            warn!("failed to subscribe to scanner stable-file events: {e:?}");
            return;
        }
    };

    loop {
        match subscriber.recv().await {
            Ok(ScanEvent::StableFile(stable_file)) => {
                emit_placeholder(stable_file);
            }
            Err(RecvError::Lagged(skipped)) => {
                warn!("scan stable subscriber lagged; skipped {skipped} events");
            }
            Err(RecvError::Closed) => break,
        }
    }
}

fn emit_placeholder(stable_file: crate::scan::collection::StableFile) {
    info!(
        file_path = %stable_file.file.path().display(),
        file_modified_at = %stable_file.modified_at,
        "upload candidate stable (M2 placeholder sink)"
    );
}
