//! The calibration uploader: drain order, backoff, dropping, reconnect,
//! serving intake while busy, and shutdown.

// standard crates
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::mocks::calibration_sender::{MockCalibrationSender, Step};
use crate::mocks::error::SleepController;
use crate::test_utils::calibrations::{layout, report};
use crate::test_utils::filesys::dirs as test_dirs;
use miru_agent::calibrations::{
    CalibrationsErr, Queue, QueueSnapshotFile, Uploader, UploaderArgs, UploaderOptions,
};
use miru_agent::disk::layout::CalibrationsLayout;
use miru_agent::errors::{Code, Error, HTTPCode};
use miru_agent::filesys::{dirs, files, File, PathExt, WriteOptions};
use miru_agent::queue::Persistence;
use miru_agent::sync::syncer::{CooldownEnd, SyncEvent};

// external crates
use futures::FutureExt;
use serde_json::json;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use uuid::Uuid;

/// Fails a test that would otherwise hang.
const HANG_GUARD: Duration = Duration::from_secs(60);

struct Harness {
    _dir: test_dirs::TempDir,
    layout: CalibrationsLayout,
    sender: Arc<MockCalibrationSender>,
    started: mpsc::UnboundedReceiver<()>,
    /// Dropping it ends the uploader's event stream; `None` before a spawn
    /// spawns without one.
    events: Option<watch::Sender<SyncEvent>>,
    sleep: Arc<SleepController>,
    uploader: Option<Uploader>,
    handle: Option<JoinHandle<()>>,
}

impl Harness {
    fn new() -> Self {
        let dir = test_dirs::temp("calibration_uploader_test").unwrap();
        let layout = layout(&dir);
        let (sender, started) = MockCalibrationSender::new();
        let (events, _) = watch::channel(SyncEvent::SyncSuccess);
        Self {
            _dir: dir,
            layout,
            sender,
            started,
            events: Some(events),
            sleep: Arc::new(SleepController::new()),
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
        let args = UploaderArgs {
            snapshot_file: Some(self.snapshot_file().await),
            reports: self.layout.reports(),
            sender: self.sender.clone(),
            events: self.events.as_ref().map(watch::Sender::subscribe),
            options,
            sleep_fn: self.sleep.sleep_fn(),
        };
        let (uploader, handle) = Uploader::spawn(64, args);
        self.uploader = Some(uploader);
        self.handle = Some(handle);
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

    fn sync(&self, event: SyncEvent) {
        self.events.as_ref().unwrap().send(event).unwrap();
    }

    fn report_file(&self, id: Uuid) -> File {
        self.layout.reports().file(&format!("{id}.json"))
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
/// would have left them: metadata in `queue.json`, content in `reports/`.
async fn preloaded(h: &Harness, slugs: &[&str]) -> Vec<Uuid> {
    let mut queue = Queue::from_snapshot(1000, h.snapshot_file().await, Persistence::Durable);
    let mut ids = Vec::with_capacity(slugs.len());
    for slug in slugs {
        let report = report(slug);
        let id = queue.enqueue(report.metadata).await.unwrap();
        let file = h.report_file(id);
        files::write_json(&file, &report.content, WriteOptions::OVERWRITE_ATOMIC)
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
async fn intake_during_a_backoff_does_not_extend_it() {
    let mut h = Harness::new();
    h.sender.push(Step::Server);
    h.spawn().await;
    h.enqueue("a").await;
    h.await_sleep().await;

    h.enqueue("b").await;

    assert_eq!(h.len().await, 2);
    assert_eq!(h.sender.sent(), strings(&["a"]));
    h.sleep.release().await;
    h.await_drained(&["a", "a", "b"]).await;
    assert_eq!(h.sleeps_secs(), [10]);
}

#[tokio::test]
async fn a_full_queue_is_507_and_keeps_what_it_holds() {
    let mut h = Harness::new();
    let (_hold, hang) = oneshot::channel::<()>();
    h.sender.push(Step::Hang(hang));
    let options = UploaderOptions {
        capacity: 1,
        ..UploaderOptions::default()
    };
    h.spawn_with(options).await;
    h.enqueue("a").await;
    h.await_started().await;

    let err = h.uploader().enqueue(report("b")).await.unwrap_err();

    assert!(matches!(err, CalibrationsErr::QueueFullErr(_)), "{err:?}");
    assert_eq!(err.code().as_str(), "calibration_buffer_full");
    assert_eq!(err.http_status(), HTTPCode::INSUFFICIENT_STORAGE);
    assert_eq!(err.params(), Some(json!({ "max_reports": 1 })));
    assert_eq!(h.len().await, 1);
}

#[test]
fn default_options_are_1000_reports_of_2_mib() {
    let options = UploaderOptions::default();
    assert_eq!(options.capacity, 1000);
    assert_eq!(options.max_report_bytes, 2 * 1024 * 1024);
}

#[tokio::test]
async fn an_oversized_report_is_413_and_writes_nothing() {
    let mut h = Harness::new();
    let options = UploaderOptions {
        max_report_bytes: 100,
        ..UploaderOptions::default()
    };
    h.spawn_with(options).await;
    let mut big = report("arm");
    big.content = json!({ "format": "text", "data": "x".repeat(200) });

    let err = h.uploader().enqueue(big).await.unwrap_err();

    assert!(matches!(err, CalibrationsErr::TooLargeErr(_)), "{err:?}");
    assert!(matches!(err.code(), Code::CalibrationTooLarge));
    assert_eq!(err.code().as_str(), "calibration_too_large");
    assert_eq!(err.http_status(), HTTPCode::PAYLOAD_TOO_LARGE);
    assert_eq!(err.params(), Some(json!({ "max_bytes": 100 })));
    assert_eq!(h.len().await, 0);
    assert!(!h.layout.reports().exists());
}

#[tokio::test]
async fn a_report_file_write_failure_queues_nothing() {
    let mut h = Harness::new();
    // a regular file where the reports directory should be
    let blocker = File::new(h.layout.reports().path().clone());
    files::write_string(&blocker, "x", WriteOptions::OVERWRITE_ATOMIC)
        .await
        .unwrap();
    h.spawn().await;

    let err = h.uploader().enqueue(report("a")).await.unwrap_err();

    assert!(matches!(err, CalibrationsErr::FileSysErr(_)), "{err:?}");
    assert!(matches!(err.code(), Code::InternalServerError), "{err}");
    assert_eq!(h.len().await, 0);
}

#[tokio::test]
async fn a_queue_write_failure_queues_nothing() {
    let mut h = Harness::new();
    h.spawn().await;
    // a non-empty directory where the queue file should be can't be replaced
    files::delete(&h.layout.queue()).await.unwrap();
    let blocker = h.layout.root.subdir("queue.json");
    files::write_string(&blocker.file("x"), "x", WriteOptions::OVERWRITE_ATOMIC)
        .await
        .unwrap();

    let err = h.uploader().enqueue(report("a")).await.unwrap_err();

    assert!(matches!(err, CalibrationsErr::FileSysErr(_)), "{err:?}");
    assert_eq!(h.len().await, 0);
    assert!(!h.layout.reports().exists());
}

/// A file whose name isn't UTF-8 can't be a report file, so the sweep
/// deletes it too.
#[cfg(unix)]
#[tokio::test]
async fn a_file_with_a_non_utf8_name_is_deleted_at_start() {
    use std::os::unix::ffi::OsStrExt;

    let mut h = Harness::new();
    preloaded(&h, &["a"]).await;
    let name = std::ffi::OsStr::from_bytes(b"\xff.json");
    let odd = File::new(h.layout.reports().path().join(name));
    files::write_string(&odd, "{}", WriteOptions::OVERWRITE_ATOMIC)
        .await
        .unwrap();

    h.spawn().await;

    h.await_drained(&["a"]).await;
    assert!(!odd.exists());
}

/// A report file that can't be deleted is left for the next boot's sweep;
/// the report itself is still dequeued.
#[cfg(unix)]
#[tokio::test]
async fn a_report_file_that_cannot_be_deleted_still_dequeues_the_report() {
    use std::os::unix::fs::PermissionsExt;

    let mut h = Harness::new();
    preloaded(&h, &["a"]).await;
    let reports = h.layout.reports();
    let mode = |m| std::fs::set_permissions(reports.path(), std::fs::Permissions::from_mode(m));
    mode(0o555).unwrap();

    h.spawn().await;
    h.await_drained(&["a"]).await;

    mode(0o755).unwrap();
    assert_eq!(h.queued_on_disk().await, 0);
}

#[tokio::test]
async fn stores_metadata_in_the_queue_and_content_in_a_report_file() {
    let mut h = Harness::new();
    let (_hold, hang) = oneshot::channel::<()>();
    h.sender.push(Step::Hang(hang));
    h.spawn().await;

    let id = h.enqueue("a").await;

    let snapshot: serde_json::Value = files::read_json(&h.layout.queue()).await.unwrap();
    assert_eq!(snapshot["entries"][0]["id"], json!(id.to_string()));
    assert_eq!(snapshot["entries"][0]["job"]["component_slug"], json!("a"));
    let content: serde_json::Value = files::read_json(&h.report_file(id)).await.unwrap();
    assert_eq!(content, report("a").content);
}

#[tokio::test]
async fn report_files_with_no_queue_entry_are_deleted_at_start() {
    let mut h = Harness::new();
    preloaded(&h, &["a"]).await;
    let stray = h.report_file(Uuid::new_v4());
    let other = h.layout.reports().file("notes.txt");
    for file in [&stray, &other] {
        files::write_string(file, "{}", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();
    }

    h.spawn().await;

    h.await_drained(&["a"]).await;
    assert!(dirs::files(&h.layout.reports()).await.unwrap().is_empty());
}

#[tokio::test]
async fn offline_then_online_uploads_in_order() {
    let mut h = Harness::new();
    h.sender.push(Step::Network);
    preloaded(&h, &["a", "b"]).await;
    h.spawn().await;

    h.await_sleep().await;
    assert_eq!(h.sleeps_secs(), [10]);
    assert_eq!(h.len().await, 2);

    // a successful sync ends the wait without the sleep being released
    h.sync(SyncEvent::SyncSuccess);

    h.await_drained(&["a", "a", "b"]).await;
    assert_eq!(h.sleeps_secs(), [10]);
}

#[tokio::test]
async fn other_sync_events_do_not_end_a_wait() {
    let mut h = Harness::new();
    h.sender.push(Step::Network);
    h.spawn().await;
    h.enqueue("a").await;
    h.await_sleep().await;

    h.sync(SyncEvent::CooldownEnd(CooldownEnd::SyncSuccess));
    tokio::time::sleep(Duration::from_millis(20)).await;

    assert_eq!(h.sender.sent(), strings(&["a"]));
    h.await_sleep().await;
    h.sleep.release().await;
    h.await_drained(&["a", "a"]).await;
}

#[tokio::test]
async fn sync_events_while_idle_are_ignored() {
    let mut h = Harness::new();
    h.spawn().await;

    h.sync(SyncEvent::SyncSuccess);
    h.sync(SyncEvent::CooldownEnd(CooldownEnd::SyncSuccess));
    h.enqueue("a").await;

    h.await_drained(&["a"]).await;
}

#[tokio::test]
async fn retryable_errors_back_off_within_bounds() {
    let mut h = Harness::new();
    for _ in 0..11 {
        h.sender.push(Step::Server);
    }
    h.spawn().await;
    h.enqueue("a").await;

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
    let mut h = Harness::new();
    for _ in 0..3 {
        h.sender.push(Step::Network);
    }
    h.spawn().await;
    h.enqueue("a").await;

    for _ in 0..3 {
        h.await_sleep().await;
        h.sleep.release().await;
    }

    h.await_drained(&["a"; 4]).await;
    assert_eq!(h.sleeps_secs(), [10, 10, 10]);
}

#[tokio::test]
async fn a_success_resets_the_backoff() {
    let mut h = Harness::new();
    h.sender.push(Step::Server);
    h.sender.push(Step::Ok);
    h.sender.push(Step::Server);
    preloaded(&h, &["a", "b"]).await;
    h.spawn().await;

    for _ in 0..2 {
        h.await_sleep().await;
        h.sleep.release().await;
    }

    h.await_drained(&["a", "a", "b", "b"]).await;
    assert_eq!(h.sleeps_secs(), [10, 10]);
}

#[tokio::test]
async fn terminal_rejection_is_dropped_not_retried() {
    let mut h = Harness::new();
    h.sender.push(Step::Terminal);
    h.spawn().await;
    let id = h.enqueue("a").await;

    h.await_drained(&["a"]).await;
    assert!(!h.report_file(id).exists());

    h.enqueue("b").await;

    h.await_drained(&["a", "b"]).await;
    assert!(h.sleeps_secs().is_empty());
}

#[tokio::test]
async fn an_unreadable_report_is_dropped_without_sending() {
    let mut h = Harness::new();
    let ids = preloaded(&h, &["a", "b"]).await;
    files::write_string(
        &h.report_file(ids[0]),
        "not json",
        WriteOptions::OVERWRITE_ATOMIC,
    )
    .await
    .unwrap();
    h.spawn().await;

    h.await_drained(&["b"]).await;
    assert!(!h.report_file(ids[0]).exists());
}

#[tokio::test]
async fn a_report_whose_file_is_missing_is_dropped() {
    let mut h = Harness::new();
    let ids = preloaded(&h, &["a", "b"]).await;
    files::delete(&h.report_file(ids[0])).await.unwrap();
    h.spawn().await;

    h.await_drained(&["b"]).await;
    assert_eq!(h.len().await, 0);
}

#[tokio::test]
async fn a_read_error_is_retried() {
    let mut h = Harness::new();
    let ids = preloaded(&h, &["a"]).await;
    // a directory where the report file should be cannot be read
    files::delete(&h.report_file(ids[0])).await.unwrap();
    let dir = h.layout.reports().subdir(format!("{}.json", ids[0]));
    dirs::create(&dir).await.unwrap();
    h.spawn().await;

    h.await_sleep().await;
    assert_eq!(h.sleeps_secs(), [10]);
    assert!(h.sender.sent().is_empty());
    assert_eq!(h.len().await, 1);

    dirs::delete(&dir).await.unwrap();
    files::write_json(
        &h.report_file(ids[0]),
        &report("a").content,
        WriteOptions::OVERWRITE_ATOMIC,
    )
    .await
    .unwrap();
    h.sleep.release().await;

    h.await_drained(&["a"]).await;
}

#[tokio::test]
async fn shutdown_mid_upload_keeps_the_report_for_replay() {
    let mut h = Harness::new();
    let (_hold, hang) = oneshot::channel();
    h.sender.push(Step::Hang(hang));
    h.spawn().await;
    h.enqueue("a").await;
    h.await_started().await;

    h.stop().await;

    assert_eq!(h.queued_on_disk().await, 1);
    // the next boot sends it again: two sends in total, which Miru may store
    // twice (dedup is out of scope)
    h.spawn().await;
    h.await_drained(&["a", "a"]).await;
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
async fn drains_without_sync_events() {
    let mut h = Harness::new();
    h.events = None;
    h.spawn().await;

    h.enqueue("a").await;

    h.await_drained(&["a"]).await;
}

#[tokio::test]
async fn an_ended_event_stream_does_not_cut_a_wait_short() {
    let mut h = Harness::new();
    h.sender.push(Step::Server);
    h.spawn().await;
    h.enqueue("a").await;
    h.await_sleep().await;

    h.events = None;
    tokio::time::sleep(Duration::from_millis(20)).await;

    assert_eq!(h.sender.sent(), strings(&["a"]));
    h.await_sleep().await;
    h.sleep.release().await;
    h.await_drained(&["a", "a"]).await;
}

#[tokio::test]
async fn an_ended_event_stream_does_not_stop_an_idle_uploader() {
    let mut h = Harness::new();
    h.spawn().await;
    assert_eq!(h.len().await, 0);

    h.events = None;
    tokio::time::sleep(Duration::from_millis(20)).await;
    h.enqueue("a").await;

    h.await_drained(&["a"]).await;
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
