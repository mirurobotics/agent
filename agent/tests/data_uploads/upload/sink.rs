// standard crates
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::mocks::{clock::TestClock, deleter::MockDeleter, upload_executor::MockUploadExecutor};
use miru_agent::data_uploads::scan::{scanner::StableFile, StableFileSink};
use miru_agent::data_uploads::upload::{
    Job, UploadStableFileSink, Uploader, UploaderExt, UploaderOptions,
};
use miru_agent::filesys::File;
use miru_agent::models::{FileRule, FileRuleRetention, FileRuleUpload};

// external crates
use chrono::DateTime;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;
use tokio::time::timeout;

const TEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Await `fut` with a timeout so sequencing bugs surface as panics, not hangs.
async fn within<T>(fut: impl Future<Output = T>) -> T {
    timeout(TEST_TIMEOUT, fut).await.expect("test timed out")
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
    }
}

/// An upload-bearing FileRule with a retention block, so the sink's
/// rule-to-job retention stamping is observable.
fn upload_rule() -> FileRule {
    FileRule {
        upload: Some(FileRuleUpload::default()),
        retention: Some(FileRuleRetention {
            require_upload: true,
            ttl_secs: 60,
        }),
        ..FileRule::default()
    }
}

/// A retention-only FileRule (no upload block).
fn retention_only_rule() -> FileRule {
    FileRule {
        upload: None,
        ..FileRule::default()
    }
}

/// An `UploadStableFileSink` over a real `Uploader` backed by a recording
/// `MockUploadExecutor`.
struct Harness {
    executor: Arc<MockUploadExecutor>,
    started_rx: UnboundedReceiver<()>,
    uploader: Arc<Uploader>,
    sink: UploadStableFileSink,
    uploader_handle: JoinHandle<()>,
}

impl Harness {
    fn start() -> Self {
        let (executor, started_rx) = MockUploadExecutor::new();
        let (uploader, uploader_handle) = Uploader::spawn(
            16,
            executor.clone(),
            MockDeleter::new(),
            UploaderOptions::default(),
            None,
            TestClock::new(),
        )
        .unwrap();
        let uploader = Arc::new(uploader);
        let sink = UploadStableFileSink::new(uploader.clone());

        Self {
            executor,
            started_rx,
            uploader,
            sink,
            uploader_handle,
        }
    }

    /// Deliver a stable file to the sink under `rule`, awaiting it as the
    /// scanner's tick would.
    async fn deliver(&self, stable: StableFile, rule: &FileRule) {
        self.sink.on_stable_file(stable, rule).await;
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

    async fn shutdown(self) {
        within(self.uploader.shutdown()).await.unwrap();
        within(self.uploader_handle).await.unwrap();
    }
}

#[tokio::test]
async fn stable_file_becomes_upload_job() {
    let mut harness = Harness::start();

    let stable = stable_file("a.log", "dpl_1", "rule_1");
    let rule = upload_rule();
    harness.deliver(stable.clone(), &rule).await;

    let jobs = harness.await_uploads(1).await;
    assert_eq!(jobs.len(), 1);
    // retention comes from the rule, not the (retention-less) stable file
    let expected = Job {
        file: stable.file,
        size: stable.size,
        digest: stable.digest,
        mtime: stable.mtime,
        first_observed_at: stable.first_observed_at,
        last_observed_at: stable.last_observed_at,
        file_rule_id: stable.file_rule_id,
        deployment_id: stable.deployment_id,
        retention: rule.retention,
    };
    assert_eq!(expected, jobs[0]);

    harness.shutdown().await;
}

/// A retention-only rule's files are scanned and ledgered, but the sink must
/// not mint an upload job for them. Interleaved with an upload-bearing file so
/// the assertion cannot pass merely because nothing was processed.
#[tokio::test]
async fn retention_only_stable_file_becomes_no_job() {
    let mut harness = Harness::start();

    let skipped = stable_file("retention-only.log", "dpl_1", "rule_keep");
    let uploaded = stable_file("uploaded.log", "dpl_1", "rule_upload");
    harness
        .deliver(skipped.clone(), &retention_only_rule())
        .await;
    harness.deliver(uploaded.clone(), &upload_rule()).await;

    // Exactly one job is driven, and it is the upload-bearing one. Awaiting one
    // upload also orders the assertion after both deliveries were processed
    // (the retention-only delivery completed before the second was made).
    let jobs = harness.await_uploads(1).await;
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].file, uploaded.file);

    harness.shutdown().await;
}

#[tokio::test]
async fn each_stable_file_becomes_a_job_in_order() {
    let mut harness = Harness::start();

    let a = stable_file("a.log", "dpl_1", "rule_1");
    let b = stable_file("b.log", "dpl_1", "rule_1");
    harness.deliver(a.clone(), &upload_rule()).await;
    harness.deliver(b.clone(), &upload_rule()).await;

    let jobs = harness.await_uploads(2).await;
    let files: Vec<_> = jobs.iter().map(|j| j.file.clone()).collect();
    assert_eq!(files, vec![a.file, b.file]);

    harness.shutdown().await;
}

/// Enqueue failures are the sink's own problem: it logs and returns `()`. Here
/// the uploader is already shut down, so the enqueue errors — the sink call
/// must still complete without panicking or propagating anything.
#[tokio::test]
async fn enqueue_failure_is_swallowed() {
    let harness = Harness::start();

    within(harness.uploader.shutdown()).await.unwrap();

    let stable = stable_file("orphan.log", "dpl_1", "rule_1");
    harness.deliver(stable, &upload_rule()).await;

    // nothing was driven and nothing panicked.
    assert!(harness.executor.recorded_calls().is_empty());
    within(harness.uploader_handle).await.unwrap();
}
