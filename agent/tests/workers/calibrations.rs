//! The calibration upload worker: drain order, backoff, parking, reconnect
//! and shutdown.

// standard crates
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::mocks::calibration_sender::{MockCalibrationSender, Step};
use crate::mocks::{error::SleepController, syncer::MockSyncer};
use crate::test_utils::calibrations::{layout, report};
use crate::test_utils::filesys::dirs as test_dirs;
use miru_agent::calibrations::{self, SharedBuffer};
use miru_agent::disk::layout::CalibrationsLayout;
use miru_agent::filesys::{dirs, files, File, PathExt, WriteOptions};
use miru_agent::sync::syncer::SyncEvent;
use miru_agent::workers::calibrations as worker;

// external crates
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use uuid::Uuid;

/// Fails a test that would otherwise hang.
const HANG_GUARD: Duration = Duration::from_secs(60);

struct Harness {
    _dir: test_dirs::TempDir,
    layout: CalibrationsLayout,
    buffer: SharedBuffer,
    sender: Arc<MockCalibrationSender>,
    started: mpsc::UnboundedReceiver<()>,
    syncer: Arc<MockSyncer>,
    sleep: Arc<SleepController>,
}

impl Harness {
    async fn new() -> Self {
        let dir = test_dirs::temp("calibration_worker_test").unwrap();
        let layout = layout(&dir);
        let buffer = SharedBuffer::open(&layout, calibrations::DEFAULT_OPTIONS)
            .await
            .unwrap();
        let (sender, started) = MockCalibrationSender::new();
        Self {
            _dir: dir,
            layout,
            buffer,
            sender,
            started,
            syncer: Arc::new(MockSyncer::new()),
            sleep: Arc::new(SleepController::new()),
        }
    }

    /// Run the worker until the test ends.
    fn spawn(&self) -> JoinHandle<()> {
        self.spawn_until(std::future::pending())
    }

    fn spawn_until(&self, shutdown: impl Future<Output = ()> + Send + 'static) -> JoinHandle<()> {
        let buffer = self.buffer.clone();
        let sender = self.sender.clone();
        let syncer = self.syncer.clone();
        let sleep = self.sleep.clone();
        tokio::spawn(async move {
            worker::run(
                &worker::Options::default(),
                &buffer,
                sender.as_ref(),
                syncer.as_ref(),
                sleep.sleep_fn(),
                Box::pin(shutdown),
            )
            .await;
        })
    }

    async fn enqueue(&self, slug: &str) -> Uuid {
        self.buffer.enqueue(report(slug)).await.unwrap()
    }

    /// The buffered reports' slugs, oldest first.
    async fn buffered(&self) -> Vec<String> {
        let guard = self.buffer.lock().await;
        guard
            .jobs()
            .into_iter()
            .map(|job| job.component_slug)
            .collect()
    }

    fn content_file(&self, id: Uuid) -> File {
        self.layout.content().file(&format!("{id}.json"))
    }

    fn failed_file(&self, id: Uuid) -> File {
        self.layout.failed().file(&format!("{id}.json"))
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

    /// Wait until every report is sent and the buffer is empty.
    async fn await_drained(&self, sent: &[&str]) {
        eventually(|| async {
            self.sender.sent() == strings(sent) && self.buffered().await.is_empty()
        })
        .await;
    }
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
async fn drains_reports_buffered_before_start_in_order() {
    let h = Harness::new().await;
    for slug in ["a", "b", "c"] {
        h.enqueue(slug).await;
    }

    let _worker = h.spawn();

    h.await_drained(&["a", "b", "c"]).await;
    assert!(dirs::files(&h.layout.content()).await.unwrap().is_empty());
    assert!(h.sleeps_secs().is_empty());
}

#[tokio::test]
async fn intake_wakes_an_idle_worker() {
    let mut h = Harness::new().await;
    let _worker = h.spawn();
    // let the worker find the buffer empty and go idle
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(h.sender.sent().is_empty());

    h.enqueue("a").await;

    h.await_started().await;
    h.await_drained(&["a"]).await;
}

#[tokio::test]
async fn offline_then_online_uploads_in_order() {
    let h = Harness::new().await;
    h.sender.push(Step::Network);
    h.enqueue("a").await;
    h.enqueue("b").await;
    let _worker = h.spawn();

    h.await_sleep().await;
    assert_eq!(h.sleeps_secs(), [10]);
    assert_eq!(h.buffered().await, strings(&["a", "b"]));

    // a successful sync ends the wait without the sleep being released
    h.syncer
        .get_transmitter()
        .send(SyncEvent::SyncSuccess)
        .unwrap();

    h.await_drained(&["a", "a", "b"]).await;
    assert_eq!(h.sleeps_secs(), [10]);
}

#[tokio::test]
async fn other_sync_events_do_not_end_a_wait() {
    let h = Harness::new().await;
    h.sender.push(Step::Network);
    h.enqueue("a").await;
    let _worker = h.spawn();
    h.await_sleep().await;

    h.syncer
        .get_transmitter()
        .send(SyncEvent::CooldownEnd(
            miru_agent::sync::syncer::CooldownEnd::SyncSuccess,
        ))
        .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;

    assert_eq!(h.sender.sent(), strings(&["a"]));
    h.await_sleep().await;
    h.sleep.release().await;
    h.await_drained(&["a", "a"]).await;
}

#[tokio::test]
async fn retryable_errors_back_off_within_bounds() {
    let h = Harness::new().await;
    for _ in 0..11 {
        h.sender.push(Step::Server);
    }
    h.enqueue("a").await;
    let _worker = h.spawn();

    for _ in 0..11 {
        h.await_sleep().await;
        h.sleep.release().await;
    }

    h.await_drained(&["a"; 12]).await;
    assert_eq!(
        h.sleeps_secs(),
        [10, 20, 40, 80, 160, 320, 640, 1280, 2560, 3600, 3600]
    );
}

#[tokio::test]
async fn network_errors_wait_a_flat_base() {
    let h = Harness::new().await;
    for _ in 0..3 {
        h.sender.push(Step::Network);
    }
    h.enqueue("a").await;
    let _worker = h.spawn();

    for _ in 0..3 {
        h.await_sleep().await;
        h.sleep.release().await;
    }

    h.await_drained(&["a"; 4]).await;
    assert_eq!(h.sleeps_secs(), [10, 10, 10]);
}

#[tokio::test]
async fn a_success_resets_the_backoff() {
    let h = Harness::new().await;
    h.sender.push(Step::Server);
    h.sender.push(Step::Ok);
    h.sender.push(Step::Server);
    h.enqueue("a").await;
    h.enqueue("b").await;
    let _worker = h.spawn();

    for _ in 0..2 {
        h.await_sleep().await;
        h.sleep.release().await;
    }

    h.await_drained(&["a", "a", "b", "b"]).await;
    assert_eq!(h.sleeps_secs(), [10, 10]);
}

#[tokio::test]
async fn terminal_rejection_is_parked_not_retried() {
    let h = Harness::new().await;
    h.sender.push(Step::Terminal);
    let id = h.enqueue("a").await;
    let _worker = h.spawn();

    h.await_drained(&["a"]).await;
    assert!(h.failed_file(id).exists());
    assert!(!h.content_file(id).exists());

    h.enqueue("b").await;

    h.await_drained(&["a", "b"]).await;
    assert!(h.sleeps_secs().is_empty());
}

#[tokio::test]
async fn an_unreadable_report_is_parked_without_sending() {
    let h = Harness::new().await;
    let id = h.enqueue("a").await;
    files::write_string(
        &h.content_file(id),
        "not json",
        WriteOptions::OVERWRITE_ATOMIC,
    )
    .await
    .unwrap();
    h.enqueue("b").await;
    let _worker = h.spawn();

    h.await_drained(&["b"]).await;
    let parked = files::read_string(&h.failed_file(id)).await.unwrap();
    assert_eq!(parked, "not json");
}

#[tokio::test]
async fn a_report_whose_file_is_missing_is_dropped() {
    let h = Harness::new().await;
    let id = h.enqueue("a").await;
    files::delete(&h.content_file(id)).await.unwrap();
    h.enqueue("b").await;
    let _worker = h.spawn();

    h.await_drained(&["b"]).await;
    assert!(!h.failed_file(id).exists());
}

#[tokio::test]
async fn a_failed_park_is_retried() {
    let h = Harness::new().await;
    h.sender.push(Step::Terminal);
    let id = h.enqueue("a").await;
    // a file where the failed directory should be
    files::write_string(
        &File::new(h.layout.failed().path().clone()),
        "x",
        WriteOptions::OVERWRITE_ATOMIC,
    )
    .await
    .unwrap();
    let _worker = h.spawn();

    h.await_sleep().await;
    assert_eq!(h.sleeps_secs(), [10]);
    assert!(h.content_file(id).exists());
    h.sleep.release().await;

    // the retry succeeds, since the mock's script is now empty
    h.await_drained(&["a", "a"]).await;
}

#[tokio::test]
async fn shutdown_mid_upload_keeps_the_report_for_replay() {
    let mut h = Harness::new().await;
    let (_hold, hang) = oneshot::channel();
    h.sender.push(Step::Hang(hang));
    h.enqueue("a").await;
    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    let worker = h.spawn_until(async move {
        let _ = stop_rx.await;
    });
    h.await_started().await;

    stop_tx.send(()).unwrap();

    guard(worker).await.unwrap();
    let reopened = SharedBuffer::open(&h.layout, calibrations::DEFAULT_OPTIONS)
        .await
        .unwrap();
    assert_eq!(reopened.lock().await.len(), 1);
    // the next boot sends it again: two sends in total, which Miru may store
    // twice (dedup is out of scope)
    let replay = Harness {
        buffer: reopened,
        ..Harness::new().await
    };
    let _worker = replay.spawn();
    replay.await_drained(&["a"]).await;
    assert_eq!(h.sender.sent().len() + replay.sender.sent().len(), 2);
}

#[tokio::test]
async fn drains_without_a_syncer_subscription() {
    let h = Harness::new().await;
    h.syncer.set_subscribe_fail(true);
    h.enqueue("a").await;

    let _worker = h.spawn();

    h.await_drained(&["a"]).await;
}

#[tokio::test]
async fn an_ended_event_stream_does_not_cut_a_wait_short() {
    let h = Harness::new().await;
    h.sender.push(Step::Server);
    h.enqueue("a").await;
    let _worker = h.spawn();
    h.await_sleep().await;

    h.syncer.drop_transmitter();
    tokio::time::sleep(Duration::from_millis(20)).await;

    assert_eq!(h.sender.sent(), strings(&["a"]));
    h.await_sleep().await;
    h.sleep.release().await;
    h.await_drained(&["a", "a"]).await;
}

#[tokio::test]
async fn an_ended_event_stream_does_not_stop_an_idle_worker() {
    let h = Harness::new().await;
    let _worker = h.spawn();
    tokio::time::sleep(Duration::from_millis(20)).await;

    h.syncer.drop_transmitter();
    tokio::time::sleep(Duration::from_millis(20)).await;
    h.enqueue("a").await;

    h.await_drained(&["a"]).await;
}

#[tokio::test]
async fn a_read_error_is_retried() {
    let h = Harness::new().await;
    let id = h.enqueue("a").await;
    // a directory where the report file should be cannot be read
    files::delete(&h.content_file(id)).await.unwrap();
    let dir = h.layout.content().subdir(format!("{id}.json"));
    dirs::create(&dir).await.unwrap();
    let _worker = h.spawn();

    h.await_sleep().await;
    assert_eq!(h.sleeps_secs(), [10]);
    assert!(h.sender.sent().is_empty());
    assert_eq!(h.buffered().await, strings(&["a"]));

    dirs::delete(&dir).await.unwrap();
    files::write_json(
        &h.content_file(id),
        &report("a"),
        WriteOptions::OVERWRITE_ATOMIC,
    )
    .await
    .unwrap();
    h.sleep.release().await;

    h.await_drained(&["a"]).await;
}
