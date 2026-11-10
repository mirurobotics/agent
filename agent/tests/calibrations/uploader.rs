//! The calibration uploader actor: drain order, serving intake while busy
//! or backing off, keeping the latest store in step with the queue, and
//! shutdown. `UploaderImpl`'s queue, storage and retry rules are unit-tested in
//! `uploader_impl.rs`.

// standard crates
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// internal crates
use crate::mocks::calibration_sender::{MockCalibrationSender, Step};
use crate::mocks::error::SleepController;
use crate::test_utils::calibrations::{
    latest_cals_layout, latest_store, layout, received_at, report,
};
use crate::test_utils::filesys::dirs as test_dirs;
use miru_agent::calibrations::{
    latest, CalibrationsErr, Filter, LatestStore, Queue, QueueSnapshotFile, Report,
    ReportMetadata, Uploader, UploaderArgs, UploaderOptions,
};
use miru_agent::disk::layout::{CalibrationsLayout, LatestCalsLayout};
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
    dir: test_dirs::TempDir,
    layout: CalibrationsLayout,
    latest_layout: LatestCalsLayout,
    sender: Arc<MockCalibrationSender>,
    started: mpsc::UnboundedReceiver<()>,
    sleep: Arc<SleepController>,
    /// The uploader's clock. It starts when the fixture reports were received
    /// and moves only when a backoff sleep completes.
    clock: Arc<Mutex<DateTime<Utc>>>,
    uploader: Option<Uploader>,
    /// The latest store the uploader writes.
    store: Option<LatestStore>,
    handle: Option<JoinHandle<()>>,
}

impl Harness {
    fn new() -> Self {
        let dir = test_dirs::temp("cal_uploader_test").unwrap();
        let layout = layout(&dir);
        let latest_layout = latest_cals_layout(&dir);
        let (sender, started) = MockCalibrationSender::new();
        Self {
            dir,
            layout,
            latest_layout,
            sender,
            started,
            sleep: Arc::new(SleepController::new()),
            clock: Arc::new(Mutex::new(received_at())),
            uploader: None,
            store: None,
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

    /// Run a calibration uploader on the queue at the layout until the test ends.
    async fn spawn(&mut self) {
        self.spawn_with(UploaderOptions::default()).await;
    }

    async fn spawn_with(&mut self, options: UploaderOptions) {
        let snapshot_file = Some(self.snapshot_file().await);
        let latest = latest_store(&self.dir, latest::DEFAULT_CAPACITY).await;
        let args = UploaderArgs {
            snapshot_file,
            reports: self.layout.reports(),
            latest: latest.clone(),
            sender: self.sender.clone(),
            options,
            sleep_fn: self.sleep_fn(),
            now_fn: self.now_fn(),
        };
        let (uploader, handle) = Uploader::spawn(64, args);
        self.uploader = Some(uploader);
        self.store = Some(latest);
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

    async fn report(&self, slug: &str) {
        guard(self.uploader().report(report(slug))).await.unwrap();
    }

    async fn num_queued_uploads(&self) -> usize {
        guard(self.uploader().num_queued_uploads()).await.unwrap()
    }

    /// Every latest entry.
    async fn latest_entries(&self) -> Vec<ReportMetadata> {
        let store = self.store.as_ref().expect("spawned");
        store.list(&Filter::default()).await.unwrap()
    }

    /// The data the latest store holds for `report_id`.
    async fn latest_data(&self, report_id: Uuid) -> String {
        let store = self.store.as_ref().expect("spawned");
        store.get(report_id).await.unwrap().data
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
        eventually(|| async {
            self.sender.sent() == strings(sent) && self.num_queued_uploads().await == 0
        })
        .await;
    }

    async fn stop(&mut self) {
        guard(self.uploader().shutdown()).await.unwrap();
        guard(self.handle.take().unwrap()).await.unwrap();
    }
}

/// Queue a report per slug at `h`'s layout, oldest first, as a previous run
/// would have left them: metadata in `queue.json`, data in `reports/`.
async fn preloaded(h: &Harness, slugs: &[&str]) -> Vec<Uuid> {
    preloaded_reports(h, slugs.iter().map(|slug| report(slug)).collect()).await
}

/// Queue `reports` at `h`'s layout, as `preloaded` does.
async fn preloaded_reports(h: &Harness, reports: Vec<Report>) -> Vec<Uuid> {
    let mut queue = Queue::from_snapshot(1000, h.snapshot_file().await, Persistence::Durable);
    let mut ids = Vec::with_capacity(reports.len());
    for report in reports {
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

/// `slug`'s fixture report, reported `secs` after it, under a new `report_id`.
fn reported_after(slug: &str, secs: i64) -> ReportMetadata {
    let metadata = report(slug).metadata;
    ReportMetadata {
        report_id: Uuid::new_v4(),
        reported_at: metadata.reported_at + TimeDelta::seconds(secs),
        ..metadata
    }
}

#[tokio::test]
async fn startup_stores_queued_reports_the_store_lacks() {
    let mut h = Harness::new();
    let newer_b = reported_after("b", 1);
    let seeded = latest_store(&h.dir, latest::DEFAULT_CAPACITY).await;
    seeded.put(reported_after("a", -1), "old a").await.unwrap();
    seeded.put(newer_b.clone(), "new b").await.unwrap();
    seeded.shutdown().await.unwrap();
    let ids = preloaded(&h, &["c", "a", "b", "d"]).await;
    // c's data is gone, so it can't be stored
    files::delete(&h.report_file(ids[0])).await.unwrap();
    let (_hold, hang) = oneshot::channel();
    h.sender.push(Step::Hang(hang));

    h.spawn().await;

    // the worker stores queued reports before it starts the first upload
    h.await_started().await;
    let expected = [report("a").metadata, newer_b.clone(), report("d").metadata];
    assert_eq!(h.latest_entries().await, expected);
    assert_eq!(h.latest_data(newer_b.report_id).await, "new b");
    assert_eq!(h.latest_data(report("d").metadata.report_id).await, report("d").data);
}

#[tokio::test]
async fn startup_stores_the_newest_of_a_keys_queued_reports() {
    let mut h = Harness::new();
    let newer_a = Report {
        metadata: reported_after("a", 1),
        data: "new a".to_string(),
    };
    preloaded_reports(&h, vec![newer_a.clone(), report("a")]).await;
    let (_hold, hang) = oneshot::channel();
    h.sender.push(Step::Hang(hang));

    h.spawn().await;

    h.await_started().await;
    assert_eq!(h.latest_entries().await, [newer_a.metadata]);
}

#[tokio::test]
async fn startup_still_uploads_when_the_store_is_unreadable() {
    let mut h = Harness::new();
    let index = h.latest_layout.index();
    files::write_string(&index, "not json", WriteOptions::OVERWRITE_ATOMIC)
        .await
        .unwrap();
    preloaded(&h, &["a"]).await;
    let (_hold, hang) = oneshot::channel();
    h.sender.push(Step::Hang(hang));

    h.spawn().await;

    h.await_started().await;
    let content = files::read_string(&index).await.unwrap();
    assert_eq!(content, "not json");
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
    assert_eq!(h.num_queued_uploads().await, 0);

    h.report("a").await;

    h.await_started().await;
    h.await_drained(&["a"]).await;
}

#[tokio::test]
async fn intake_is_served_during_an_upload() {
    let mut h = Harness::new();
    let (release, hang) = oneshot::channel();
    h.sender.push(Step::Hang(hang));
    h.spawn().await;
    h.report("a").await;
    h.await_started().await;

    // the send of "a" is still in flight
    h.report("b").await;
    assert_eq!(h.num_queued_uploads().await, 2);

    release.send(()).unwrap();
    h.await_drained(&["a", "b"]).await;
}

#[tokio::test]
async fn a_failed_store_write_queues_nothing() {
    let mut h = Harness::new();
    // a report left queued would hang here rather than upload and leave
    let (_hold, hang) = oneshot::channel();
    h.sender.push(Step::Hang(hang));
    h.spawn().await;
    // a regular file where the latest contents directory should be
    let blocker = File::new(h.latest_layout.contents().path().clone());
    files::write_string(&blocker, "x", WriteOptions::OVERWRITE_ATOMIC)
        .await
        .unwrap();

    let err = guard(h.uploader().report(report("a"))).await.unwrap_err();

    assert!(matches!(err, CalibrationsErr::FileSysErr(_)), "{err:?}");
    assert_eq!(h.num_queued_uploads().await, 0);
    assert_eq!(h.queued_on_disk().await, 0);
    assert!(dirs::files(&h.layout.reports()).await.unwrap().is_empty());
    assert!(h.sender.sent().is_empty());
}

#[tokio::test]
async fn a_rejected_report_is_not_stored() {
    let mut h = Harness::new();
    let (_hold, hang) = oneshot::channel();
    h.sender.push(Step::Hang(hang));
    let options = UploaderOptions {
        capacity: 1,
        ..UploaderOptions::default()
    };
    h.spawn_with(options).await;
    h.report("a").await;

    let err = guard(h.uploader().report(report("b"))).await.unwrap_err();

    assert!(matches!(err, CalibrationsErr::QueueFullErr(_)), "{err:?}");
    assert_eq!(h.latest_entries().await, [report("a").metadata]);
}

#[tokio::test]
async fn an_oversized_report_is_not_stored() {
    let mut h = Harness::new();
    let options = UploaderOptions {
        max_report_bytes: 1,
        ..UploaderOptions::default()
    };
    h.spawn_with(options).await;

    let err = guard(h.uploader().report(report("a"))).await.unwrap_err();

    assert!(matches!(err, CalibrationsErr::TooLargeErr(_)), "{err:?}");
    assert_eq!(h.num_queued_uploads().await, 0);
    assert!(h.latest_entries().await.is_empty());
}

#[tokio::test]
async fn a_new_report_is_sent_while_another_backs_off() {
    let mut h = Harness::new();
    h.sender.push(Step::Server);
    h.spawn().await;
    h.report("a").await;
    h.await_sleep().await;

    h.report("b").await;

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
    h.report("a").await;
    h.await_started().await;

    h.stop().await;

    assert_eq!(h.queued_on_disk().await, 1);
    // the next boot sends it again with the same report_id, so Miru records it
    // once
    h.spawn().await;
    h.await_drained(&["a", "a"]).await;
    let id = report("a").metadata.report_id;
    assert_eq!(h.sender.report_ids(), [id, id]);
}

#[tokio::test]
async fn shutdown_during_a_backoff() {
    let mut h = Harness::new();
    h.sender.push(Step::Server);
    h.spawn().await;
    h.report("a").await;
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

    let err = h.uploader().report(report("a")).await.unwrap_err();

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
    assert!(h.uploader().num_queued_uploads().now_or_never().is_none());

    h.report("a").await;
    h.await_drained(&["a"]).await;
}

#[tokio::test]
async fn a_command_queued_behind_shutdown_fails() {
    let mut h = Harness::new();
    h.spawn().await;

    let uploader = h.uploader();
    let (stopped, enqueued) = tokio::join!(uploader.shutdown(), uploader.report(report("a")));

    stopped.unwrap();
    let err = enqueued.unwrap_err();
    assert!(
        matches!(err, CalibrationsErr::ReceiveActorMessageErr(_)),
        "{err:?}"
    );
    guard(h.handle.take().unwrap()).await.unwrap();
    assert_eq!(h.queued_on_disk().await, 0);
}
