// standard crates
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::mocks::{scanner::MockScanner, upload_executor::MockUploadExecutor};
use miru_agent::filesys::File;
use miru_agent::models::{FileRule, FileRuleRetention, FileRuleUpload};
use miru_agent::scan::scanner::StableFile;
use miru_agent::scan::ScanEvent;
use miru_agent::upload::{Job, Uploader, UploaderExt, UploaderOptions};
use miru_agent::workers::scan_upload_bridge;

// external crates
use chrono::DateTime;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::timeout;

const TEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Await `fut` with a timeout so sequencing bugs surface as panics, not hangs.
async fn within<T>(fut: impl Future<Output = T>) -> T {
    timeout(TEST_TIMEOUT, fut).await.expect("test timed out")
}

/// Poll `cond` until it holds, yielding between attempts; panics after a bounded
/// timeout so a broken worker cannot hang the suite.
async fn wait_until<F: Fn() -> bool>(cond: F) {
    let poll = async {
        while !cond() {
            tokio::task::yield_now().await;
        }
    };
    within(poll).await;
}

fn stable_file(name: &str, deployment_id: &str, rule_id: &str) -> StableFile {
    StableFile {
        file: File::new(format!("/data/{name}")),
        size: 128,
        digest: format!("sha256:{name}"),
        mtime: DateTime::from_timestamp(1000, 0).unwrap(),
        mtime_aliases: Vec::new(),
        first_observed_at: DateTime::from_timestamp(900, 0).unwrap(),
        last_observed_at: DateTime::from_timestamp(1000, 0).unwrap(),
        deployment_id: deployment_id.to_string(),
        file_rule_id: rule_id.to_string(),
        retention: Some(FileRuleRetention {
            require_upload: true,
            ttl_secs: 0,
        }),
    }
}

/// A running scan-upload bridge over a `MockScanner` (drives events) and a real
/// `Uploader` backed by a recording `MockUploadExecutor`.
struct Harness {
    scanner: Arc<MockScanner>,
    executor: Arc<MockUploadExecutor>,
    started_rx: UnboundedReceiver<()>,
    uploader: Arc<Uploader>,
    bridge: JoinHandle<()>,
    bridge_shutdown: Option<oneshot::Sender<()>>,
    uploader_handle: JoinHandle<()>,
}

impl Harness {
    async fn start() -> Self {
        let (executor, started_rx) = MockUploadExecutor::new();
        let (uploader, uploader_handle) = Uploader::spawn(
            16,
            executor.clone(),
            UploaderOptions::default(),
            None,
            |_: Duration| async {},
            chrono::Utc::now,
        )
        .unwrap();
        let uploader = Arc::new(uploader);

        let scanner = Arc::new(MockScanner::new());

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let scanner_task = scanner.clone();
        let uploader_task = uploader.clone();
        let bridge = tokio::spawn(async move {
            scan_upload_bridge::run(
                scanner_task.as_ref(),
                uploader_task.as_ref(),
                Box::pin(async move {
                    let _ = shutdown_rx.await;
                }),
            )
            .await;
        });

        // the worker subscribes at startup; wait for it so emitted events are
        // delivered rather than dropped by the broadcast channel
        wait_until(|| scanner.subscriber_count() > 0).await;

        Self {
            scanner,
            executor,
            started_rx,
            uploader,
            bridge,
            bridge_shutdown: Some(shutdown_tx),
            uploader_handle,
        }
    }

    /// Emit a stable file from an upload-bearing rule.
    fn emit(&self, stable: StableFile) {
        self.scanner.emit(ScanEvent::StableFile {
            file: stable,
            rule: FileRule {
                upload: Some(FileRuleUpload::default()),
                ..FileRule::default()
            },
        });
    }

    /// Emit a stable file from a retention-only rule (no upload block).
    fn emit_retention_only(&self, stable: StableFile) {
        self.scanner.emit(ScanEvent::StableFile {
            file: stable,
            rule: FileRule {
                upload: None,
                ..FileRule::default()
            },
        });
    }

    /// Wait until the executor has been driven `n` times (it records each job
    /// before signalling), then return the recorded jobs in order.
    async fn await_uploads(&mut self, n: usize) -> Vec<Job> {
        for _ in 0..n {
            within(self.started_rx.recv())
                .await
                .expect("executor should have been driven");
        }
        self.executor.recorded_calls()
    }

    async fn shutdown(mut self) {
        if let Some(tx) = self.bridge_shutdown.take() {
            let _ = tx.send(());
        }
        within(self.bridge).await.unwrap();
        within(self.uploader.shutdown()).await.unwrap();
        within(self.uploader_handle).await.unwrap();
    }
}

#[tokio::test]
async fn stable_file_becomes_upload_job() {
    let mut harness = Harness::start().await;

    let stable = stable_file("a.log", "dpl_1", "rule_1");
    harness.emit(stable.clone());

    let jobs = harness.await_uploads(1).await;
    assert_eq!(jobs.len(), 1);
    let expected = Job {
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
    assert_eq!(expected, jobs[0]);

    harness.shutdown().await;
}

#[tokio::test]
async fn idles_when_scanner_stream_closes() {
    let mut harness = Harness::start().await;

    // the scanner going away closes the event stream. The bridge must NOT exit
    // on `Closed` — that would silently retire the worker while the app still
    // holds its handle and believes the pipeline is live.
    harness.scanner.close();

    // a wrongly-implemented bridge (returning on `Closed`) would complete its
    // task almost immediately; a correct one idles until the shutdown signal.
    // `Err(Elapsed)` means the handle is still pending — the desired outcome.
    assert!(
        timeout(Duration::from_millis(100), &mut harness.bridge)
            .await
            .is_err(),
        "bridge exited on stream close instead of idling until shutdown"
    );

    // the shutdown signal remains the only intended exit, and it works.
    harness.shutdown().await;
}

/// A retention-only rule's files are scanned and ledgered, but the bridge must
/// not mint an upload job for them. Interleaved with an upload-bearing file so
/// the assertion cannot pass merely because nothing was processed.
#[tokio::test]
async fn retention_only_stable_file_becomes_no_job() {
    let mut harness = Harness::start().await;

    let skipped = stable_file("retention-only.log", "dpl_1", "rule_keep");
    let uploaded = stable_file("uploaded.log", "dpl_1", "rule_upload");
    harness.emit_retention_only(skipped.clone());
    harness.emit(uploaded.clone());

    // Exactly one job is driven, and it is the upload-bearing one. Awaiting one
    // upload also orders the assertion after both events were consumed.
    let jobs = harness.await_uploads(1).await;
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].file, uploaded.file);

    harness.shutdown().await;
}

#[tokio::test]
async fn each_stable_file_becomes_a_job_in_order() {
    let mut harness = Harness::start().await;

    let a = stable_file("a.log", "dpl_1", "rule_1");
    let b = stable_file("b.log", "dpl_1", "rule_1");
    harness.emit(a.clone());
    harness.emit(b.clone());

    let jobs = harness.await_uploads(2).await;
    let files: Vec<_> = jobs.iter().map(|j| j.file.clone()).collect();
    assert_eq!(files, vec![a.file, b.file]);

    harness.shutdown().await;
}
