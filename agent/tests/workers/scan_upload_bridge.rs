// standard crates
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::mocks::scanner::MockScanner;
use crate::mocks::upload_executor::MockUploadExecutor;
use miru_agent::disk::{self, Deployments, Layout};
use miru_agent::filesys::{dirs, File};
use miru_agent::models::{Deployment, DplActivity};
use miru_agent::scan::scanner::StableFile;
use miru_agent::scan::ScanEvent;
use miru_agent::upload::{Uploader, UploaderExt, UploaderOptions};
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
        upload_rule_id: rule_id.to_string(),
    }
}

/// A running scan-upload bridge over a `MockScanner` (drives events), a real
/// `Uploader` backed by a recording `MockUploadExecutor`, and a real
/// `Deployments` store for release resolution.
struct Harness {
    scanner: Arc<MockScanner>,
    executor: Arc<MockUploadExecutor>,
    started_rx: UnboundedReceiver<()>,
    uploader: Arc<Uploader>,
    deployments: Arc<Deployments>,
    _dir: dirs::TempDir,
    bridge: JoinHandle<()>,
    bridge_shutdown: Option<oneshot::Sender<()>>,
    uploader_handle: JoinHandle<()>,
}

impl Harness {
    async fn start() -> Self {
        let dir = dirs::temp("scan_upload_bridge").unwrap();
        let layout = Layout::new(dir.to_dir());
        let (deployments, _) = disk::Deployments::spawn(64, layout.deployments(), 1000)
            .await
            .unwrap();
        let deployments = Arc::new(deployments);

        let (executor, started_rx) = MockUploadExecutor::new();
        let (uploader, uploader_handle) = Uploader::spawn(
            16,
            executor.clone(),
            UploaderOptions::default(),
            |_: Duration| async {},
        )
        .unwrap();
        let uploader = Arc::new(uploader);

        let scanner = Arc::new(MockScanner::new());

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let scanner_task = scanner.clone();
        let uploader_task = uploader.clone();
        let deployments_task = deployments.clone();
        let bridge = tokio::spawn(async move {
            scan_upload_bridge::run(
                scanner_task.as_ref(),
                uploader_task.as_ref(),
                deployments_task.as_ref(),
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
            deployments,
            _dir: dir,
            bridge,
            bridge_shutdown: Some(shutdown_tx),
            uploader_handle,
        }
    }

    async fn seed_deployment(&self, dpl_id: &str, release_id: &str) {
        self.deployments
            .write_if_absent(
                dpl_id.to_string(),
                Deployment {
                    id: dpl_id.to_string(),
                    activity_status: DplActivity::Deployed,
                    release_id: release_id.to_string(),
                    ..Default::default()
                },
                |_, _| false,
            )
            .await
            .unwrap();
    }

    fn emit(&self, stable: StableFile) {
        self.scanner.emit(ScanEvent::StableFile(stable));
    }

    /// Wait until the executor has been driven once (it records the job before
    /// signalling), then return the recorded jobs.
    async fn await_next_upload(&mut self) -> Vec<miru_agent::upload::Job> {
        within(self.started_rx.recv())
            .await
            .expect("executor should have been driven");
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
async fn stable_file_becomes_upload_job_with_resolved_release() {
    let mut harness = Harness::start().await;
    harness.seed_deployment("dpl_1", "rls_9").await;

    let stable = stable_file("a.log", "dpl_1", "rule_1");
    harness.emit(stable.clone());

    let jobs = harness.await_next_upload().await;
    assert_eq!(jobs.len(), 1);
    let job = &jobs[0];
    // fields carried straight through from the stable file
    assert_eq!(job.file, stable.file);
    assert_eq!(job.size, stable.size);
    assert_eq!(job.digest, stable.digest);
    assert_eq!(job.mtime, stable.mtime);
    assert_eq!(job.first_observed_at, stable.first_observed_at);
    assert_eq!(job.last_observed_at, stable.last_observed_at);
    assert_eq!(job.upload_rule_id, "rule_1");
    assert_eq!(job.deployment_id, "dpl_1");
    // release resolved from the deployment record on disk
    assert_eq!(job.release_id, "rls_9");

    harness.shutdown().await;
}

#[tokio::test]
async fn unresolvable_deployment_is_skipped() {
    let mut harness = Harness::start().await;
    harness.seed_deployment("dpl_1", "rls_9").await;

    // first an orphan whose deployment is not on disk: it must be skipped, not
    // enqueued. The resolvable file emitted next lands behind it, so once the
    // executor runs we know the orphan was already handled and dropped.
    harness.emit(stable_file("orphan.log", "missing_dpl", "rule_1"));
    let valid = stable_file("a.log", "dpl_1", "rule_1");
    harness.emit(valid.clone());

    let jobs = harness.await_next_upload().await;
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].file, valid.file);
    assert_eq!(jobs[0].release_id, "rls_9");

    harness.shutdown().await;
}
