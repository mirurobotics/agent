//! The calibration uploader actor: drain order, serving intake while busy
//! or backing off, and shutdown. `UploaderImpl`'s queue, storage and retry
//! rules are unit-tested in `uploader_impl.rs`.

// standard crates
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// internal crates
use crate::mocks::calibration_sender::{MockCalibrationSender, Step};
use crate::mocks::error::SleepController;
use crate::test_utils::calibrations::{layout, received_at, report};
use crate::test_utils::filesys::dirs as test_dirs;
use miru_agent::calibrations::{
    CalibrationsErr, Queue, QueueSnapshotFile, Uploader, UploaderArgs, UploaderOptions,
};
use miru_agent::disk::layout::CalibrationsLayout;
use miru_agent::filesys::{dirs, files, File, PathExt, WriteOptions};
use miru_agent::queue::Persistence;

// external crates
use chrono::{DateTime, TimeDelta, Utc};
use futures::future::BoxFuture;
use futures::FutureExt;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use uuid::Uuid;

/// Fails a test that would otherwise hang.
const HANG_GUARD: Duration = Duration::from_secs(60);

struct Harness {
    _dir: test_dirs::TempDir,
    layout: CalibrationsLayout,
    sender: Arc<MockCalibrationSender>,
    started: mpsc::UnboundedReceiver<()>,
    sleep: Arc<SleepController>,
    /// The uploader's clock. It starts when the fixture reports were received
    /// and moves only when a backoff sleep completes.
    clock: Arc<Mutex<DateTime<Utc>>>,
    uploader: Option<Uploader>,
    handle: Option<JoinHandle<()>>,
}

impl Harness {
    fn new() -> Self {
        let dir = test_dirs::temp("calibration_uploader_test").unwrap();
        let layout = layout(&dir);
        let (sender, started) = MockCalibrationSender::new();
        Self {
            _dir: dir,
            layout,
            sender,
            started,
            sleep: Arc::new(SleepController::new()),
            clock: Arc::new(Mutex::new(received_at())),
            uploader: None,
            handle: None,
        }
    }

    async fn snapshot_file(&self) -> QueueSnapshotFile {
        QueueSnapshotFile::new_with_default(self.layout.queue(), Default::default())
            .await
            .unwrap()
    }

    /// The number of reports in the queue file, as the next boot would load it.
    async fn queued_on_disk(&self) -> usize {
        Queue::from_snapshot(1000, self.snapshot_file().await, Persistence::Durable).len()
    }

    /// Run an uploader on the queue at the layout until the test ends.
    async fn spawn(&mut self) {
        self.spawn_with(UploaderOptions::default()).await;
    }

    async fn spawn_with(&mut self, options: UploaderOptions) {
        let snapshot_file = Some(self.snapshot_file().await);
        self.spawn_on(snapshot_file, options);
    }

    fn spawn_on(&mut self, snapshot_file: Option<QueueSnapshotFile>, options: UploaderOptions) {
        let args = UploaderArgs {
            snapshot_file,
            reports: self.layout.reports(),
            sender: self.sender.clone(),
            options,
            sleep_fn: self.sleep_fn(),
            now_fn: self.now_fn(),
        };
        let (uploader, handle) = Uploader::spawn(64, args);
        self.uploader = Some(uploader);
        self.handle = Some(handle);
    }

    /// The controlled sleep, advancing the clock by each sleep that completes.
    fn sleep_fn(&self) -> impl Fn(Duration) -> BoxFuture<'static, ()> + Send + 'static {
        let sleep = self.sleep.sleep_fn();
        let clock = self.clock.clone();
        move |wait| {
            let slept = sleep(wait);
            let clock = clock.clone();
            async move {
                slept.await;
                *clock.lock().unwrap() += TimeDelta::from_std(wait).unwrap();
            }
            .boxed()
        }
    }

    fn now_fn(&self) -> impl Fn() -> DateTime<Utc> + Send + Sync + 'static {
        let clock = self.clock.clone();
        move || *clock.lock().unwrap()
    }

    fn uploader(&self) -> &Uploader {
        self.uploader.as_ref().expect("spawned")
    }

    async fn enqueue(&self, slug: &str) -> Uuid {
        guard(self.uploader().enqueue(report(slug))).await.unwrap()
    }

    async fn len(&self) -> usize {
        guard(self.uploader().len()).await.unwrap()
    }

    fn report_file(&self, id: Uuid) -> File {
        self.layout.reports().file(&id.to_string())
    }

    async fn await_sleep(&self) {
        guard(self.sleep.await_sleep()).await;
    }

    fn sleeps_secs(&self) -> Vec<u64> {
        let sleeps = self.sleep.attempted_sleeps();
        sleeps.iter().map(Duration::as_secs).collect()
    }

    async fn await_started(&mut self) {
        guard(self.started.recv()).await.unwrap();
    }

    /// Wait until every report is sent and the queue is empty.
    async fn await_drained(&self, sent: &[&str]) {
        eventually(|| async { self.sender.sent() == strings(sent) && self.len().await == 0 }).await;
    }

    async fn stop(&mut self) {
        guard(self.uploader().shutdown()).await.unwrap();
        guard(self.handle.take().unwrap()).await.unwrap();
    }
}

/// Queue a report per slug at `h`'s layout, oldest first, as a previous run
/// would have left them: metadata in `queue.json`, data in `reports/`.
async fn preloaded(h: &Harness, slugs: &[&str]) -> Vec<Uuid> {
    let mut queue = Queue::from_snapshot(1000, h.snapshot_file().await, Persistence::Durable);
    let mut ids = Vec::with_capacity(slugs.len());
    for slug in slugs {
        let report = report(slug);
        let id = queue.enqueue(report.metadata).await.unwrap();
        let file = h.report_file(id);
        files::write_string(&file, &report.data, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();
        ids.push(id);
    }
    ids
}

async fn guard<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(HANG_GUARD, future)
        .await
        .expect("timed out")
}

/// Wait until `check` holds.
async fn eventually<F, Fut>(check: F)
where
    F: Fn() -> Fut,
    Fut: Future<Output = bool>,
{
    guard(async {
        while !check().await {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
}

fn strings(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| name.to_string()).collect()
}

#[tokio::test]
async fn startup_prunes_orphaned_report_files() {
    let mut h = Harness::new();
    let ids = preloaded(&h, &["a"]).await;
    let orphan = h.report_file(Uuid::new_v4());
    files::write_string(&orphan, "{}", WriteOptions::OVERWRITE_ATOMIC)
        .await
        .unwrap();
    let (_hold, hang) = oneshot::channel();
    h.sender.push(Step::Hang(hang));

    h.spawn().await;

    // the worker prunes before it starts the first upload
    h.await_started().await;
    assert!(!orphan.exists());
    assert!(h.report_file(ids[0]).exists());
}

#[tokio::test]
async fn drains_reports_buffered_before_start_in_order() {
    let mut h = Harness::new();
    preloaded(&h, &["a", "b", "c"]).await;

    h.spawn().await;

    h.await_drained(&["a", "b", "c"]).await;
    assert!(dirs::files(&h.layout.reports()).await.unwrap().is_empty());
    assert!(h.sleeps_secs().is_empty());
}

#[tokio::test]
async fn intake_wakes_an_idle_uploader() {
    let mut h = Harness::new();
    h.spawn().await;
    assert_eq!(h.len().await, 0);

    h.enqueue("a").await;

    h.await_started().await;
    h.await_drained(&["a"]).await;
}

#[tokio::test]
async fn intake_is_served_during_an_upload() {
    let mut h = Harness::new();
    let (release, hang) = oneshot::channel();
    h.sender.push(Step::Hang(hang));
    h.spawn().await;
    h.enqueue("a").await;
    h.await_started().await;

    // the send of "a" is still in flight
    h.enqueue("b").await;
    assert_eq!(h.len().await, 2);

    release.send(()).unwrap();
    h.await_drained(&["a", "b"]).await;
}

#[tokio::test]
async fn a_new_report_is_sent_while_another_backs_off() {
    let mut h = Harness::new();
    h.sender.push(Step::Server);
    h.spawn().await;
    h.enqueue("a").await;
    h.await_sleep().await;

    h.enqueue("b").await;

    eventually(|| async { h.sender.sent() == strings(&["a", "b"]) }).await;
    h.await_sleep().await;
    h.sleep.release().await;
    h.await_drained(&["a", "b", "a"]).await;
    assert_eq!(h.sleeps_secs(), [10, 10]);
}

#[tokio::test]
async fn shutdown_mid_upload_keeps_the_report_for_replay() {
    let mut h = Harness::new();
    let (_hold, hang) = oneshot::channel();
    h.sender.push(Step::Hang(hang));
    h.spawn().await;
    let id = h.enqueue("a").await;
    h.await_started().await;

    h.stop().await;

    assert_eq!(h.queued_on_disk().await, 1);
    // the next boot sends it again with the same report_id, so Miru records it
    // once
    h.spawn().await;
    h.await_drained(&["a", "a"]).await;
    assert_eq!(h.sender.report_ids(), [id, id]);
}

#[tokio::test]
async fn shutdown_during_a_backoff() {
    let mut h = Harness::new();
    h.sender.push(Step::Server);
    h.spawn().await;
    h.enqueue("a").await;
    h.await_sleep().await;

    h.stop().await;

    assert_eq!(h.queued_on_disk().await, 1);
}

#[tokio::test]
async fn dropping_every_handle_stops_the_uploader() {
    let mut h = Harness::new();
    h.spawn().await;

    h.uploader = None;

    guard(h.handle.take().unwrap()).await.unwrap();
}

#[tokio::test]
async fn commands_after_shutdown_fail() {
    let mut h = Harness::new();
    h.spawn().await;
    h.stop().await;

    let err = h.uploader().enqueue(report("a")).await.unwrap_err();

    assert!(
        matches!(err, CalibrationsErr::SendActorMessageErr(_)),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_caller_that_stops_waiting_does_not_stop_the_uploader() {
    let mut h = Harness::new();
    h.spawn().await;

    // polled once: the command is sent, then its reply is abandoned
    assert!(h.uploader().len().now_or_never().is_none());

    h.enqueue("a").await;
    h.await_drained(&["a"]).await;
}

#[tokio::test]
async fn a_command_queued_behind_shutdown_fails() {
    let mut h = Harness::new();
    h.spawn().await;

    let uploader = h.uploader();
    let (stopped, enqueued) = tokio::join!(uploader.shutdown(), uploader.enqueue(report("a")));

    stopped.unwrap();
    let err = enqueued.unwrap_err();
    assert!(
        matches!(err, CalibrationsErr::ReceiveActorMessageErr(_)),
        "{err:?}"
    );
    guard(h.handle.take().unwrap()).await.unwrap();
    assert_eq!(h.queued_on_disk().await, 0);
}
