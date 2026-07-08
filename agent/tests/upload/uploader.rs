// standard crates
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// internal crates
use crate::mocks::upload_executor::{MockStep, MockUploadExecutor};
use miru_agent::filesys::{self, dirs, files, File, WriteOptions};
use miru_agent::upload::errors::ExecutorErr;
use miru_agent::upload::{
    EnqueueOutcome, UploadErr, UploadJob, Uploader, UploaderExt, UploaderOptions,
};

// external crates
use chrono::{DateTime, Utc};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::timeout;

const TEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Await `fut` with a timeout so sequencing bugs surface as panics, not
/// hanging tests.
async fn within<T>(fut: impl Future<Output = T>) -> T {
    timeout(TEST_TIMEOUT, fut).await.expect("test timed out")
}

fn make_job(name: &str) -> UploadJob {
    UploadJob {
        file: File::new(format!("/data/{name}")),
        size: 42,
        digest: format!("sha256:{name}"),
        mtime: Utc::now(),
        upload_rule_id: "rule_1".to_string(),
        deployment_id: "dpl_1".to_string(),
        release_id: "rls_1".to_string(),
    }
}

fn scripted_err() -> Result<(), UploadErr> {
    Err(UploadErr::ExecutorErr(ExecutorErr {
        source: Box::new(std::io::Error::other("scripted failure")),
        trace: miru_agent::trace!(),
    }))
}

/// Build a job backed by a real temp file, so queue staleness checks see it
/// as fresh.
async fn make_real_job(dir: &filesys::Dir, name: &str, contents: &str) -> UploadJob {
    let file = dir.file(name);
    files::write_string(&file, contents, WriteOptions::OVERWRITE_ATOMIC)
        .await
        .unwrap();
    UploadJob {
        file: file.clone(),
        size: files::size(&file).await.unwrap(),
        digest: files::hash(&file).await.unwrap(),
        mtime: DateTime::<Utc>::from(files::last_modified(&file).await.unwrap()),
        upload_rule_id: "rule_1".to_string(),
        deployment_id: "dpl_1".to_string(),
        release_id: "rls_1".to_string(),
    }
}

/// Spawn an uploader with default options and a no-op sleep.
fn spawn_uploader(mock: Arc<MockUploadExecutor>) -> (Uploader, JoinHandle<()>) {
    Uploader::spawn(16, mock, UploaderOptions::default(), |_: Duration| async {}).unwrap()
}

#[tokio::test]
async fn processes_enqueued_job() {
    let (mock, mut started_rx) = MockUploadExecutor::new();
    let (uploader, handle) = spawn_uploader(mock.clone());
    let job = make_job("a.log");

    let outcome = within(uploader.enqueue(job.clone())).await.unwrap();
    assert_eq!(outcome, EnqueueOutcome::Enqueued);
    within(started_rx.recv()).await.unwrap();

    assert_eq!(mock.recorded_calls(), vec![job]);
    assert_eq!(within(uploader.len()).await.unwrap(), 0);

    within(uploader.shutdown()).await.unwrap();
    within(handle).await.unwrap();
}

#[tokio::test]
async fn duplicate_while_in_flight_returns_duplicate() {
    let (mock, mut started_rx) = MockUploadExecutor::new();
    let (release_tx, release_rx) = oneshot::channel();
    mock.push_step(MockStep::Hang(release_rx));
    let (uploader, handle) = spawn_uploader(mock.clone());
    let job = make_job("a.log");

    within(uploader.enqueue(job.clone())).await.unwrap();
    within(started_rx.recv()).await.unwrap();

    // key-equal despite different deployment/release ids
    let mut dup = job.clone();
    dup.deployment_id = "dpl_2".to_string();
    dup.release_id = "rls_2".to_string();
    let outcome = within(uploader.enqueue(dup)).await.unwrap();
    assert_eq!(outcome, EnqueueOutcome::Duplicate);

    release_tx.send(Ok(())).unwrap();
    within(uploader.shutdown()).await.unwrap();
    within(handle).await.unwrap();
    assert_eq!(mock.recorded_calls(), vec![job]);
}

#[tokio::test]
async fn failing_round_requeues_at_tail_behind_later_job() {
    let (mock, mut started_rx) = MockUploadExecutor::new();
    let (release_tx, release_rx) = oneshot::channel();
    mock.push_step(MockStep::Hang(release_rx));
    mock.push_step(MockStep::Err);
    mock.push_step(MockStep::Err);
    mock.push_step(MockStep::Ok);
    mock.push_step(MockStep::Ok);
    let (uploader, handle) = spawn_uploader(mock.clone());
    let job_a = make_job("a.log");
    let job_b = make_job("b.log");

    within(uploader.enqueue(job_a.clone())).await.unwrap();
    within(started_rx.recv()).await.unwrap();
    // B is queued while A is in flight, so it lands ahead of A's requeue slot
    within(uploader.enqueue(job_b.clone())).await.unwrap();
    release_tx.send(scripted_err()).unwrap();
    for _ in 0..4 {
        within(started_rx.recv()).await.unwrap();
    }

    within(uploader.shutdown()).await.unwrap();
    within(handle).await.unwrap();
    // three in-place attempts for A, then B, then A's second round succeeding
    let expected = vec![job_a.clone(), job_a.clone(), job_a.clone(), job_b, job_a];
    assert_eq!(mock.recorded_calls(), expected);
}

#[tokio::test]
async fn global_attempt_cap_drops_job() {
    let (mock, mut started_rx) = MockUploadExecutor::new();
    for _ in 0..9 {
        mock.push_step(MockStep::Err);
    }
    let (uploader, handle) = spawn_uploader(mock.clone());
    let job_a = make_job("a.log");
    let job_b = make_job("b.log");

    within(uploader.enqueue(job_a.clone())).await.unwrap();
    // three rounds of three attempts each
    for _ in 0..9 {
        within(started_rx.recv()).await.unwrap();
    }

    // A was dropped at the cap with the actor still healthy: B processes next
    within(uploader.enqueue(job_b.clone())).await.unwrap();
    within(started_rx.recv()).await.unwrap();

    within(uploader.shutdown()).await.unwrap();
    within(handle).await.unwrap();
    let mut expected = vec![job_a; 9];
    expected.push(job_b);
    assert_eq!(mock.recorded_calls(), expected);
}

#[tokio::test]
async fn retry_backoff_follows_expected_sequence() {
    let (mock, mut started_rx) = MockUploadExecutor::new();
    for _ in 0..6 {
        mock.push_step(MockStep::Err);
    }
    mock.push_step(MockStep::Ok);
    let sleeps: Arc<Mutex<Vec<Duration>>> = Arc::new(Mutex::new(Vec::new()));
    let recorded = sleeps.clone();
    let sleep_fn = move |duration: Duration| {
        recorded.lock().unwrap().push(duration);
        async {}
    };
    let (uploader, handle) =
        Uploader::spawn(16, mock.clone(), UploaderOptions::default(), sleep_fn).unwrap();

    within(uploader.enqueue(make_job("a.log"))).await.unwrap();
    for _ in 0..7 {
        within(started_rx.recv()).await.unwrap();
    }

    // in-place sleeps only (exponent = total attempts - 1); none around the
    // two requeues
    let expected = vec![
        Duration::from_secs(1),
        Duration::from_secs(2),
        Duration::from_secs(8),
        Duration::from_secs(16),
    ];
    assert_eq!(*sleeps.lock().unwrap(), expected);

    within(uploader.shutdown()).await.unwrap();
    within(handle).await.unwrap();
}

#[tokio::test]
async fn shutdown_during_in_flight_upload_returns_promptly() {
    let (mock, mut started_rx) = MockUploadExecutor::new();
    let (release_tx, release_rx) = oneshot::channel();
    mock.push_step(MockStep::Hang(release_rx));
    let (uploader, handle) = spawn_uploader(mock.clone());

    within(uploader.enqueue(make_job("a.log"))).await.unwrap();
    within(started_rx.recv()).await.unwrap();

    // the hang is never released: shutdown must drop the in-flight future
    // rather than await it
    timeout(Duration::from_secs(1), uploader.shutdown())
        .await
        .expect("shutdown timed out awaiting the in-flight upload")
        .unwrap();
    within(handle).await.unwrap();
    drop(release_tx);
}

#[tokio::test]
async fn requeue_into_full_queue_drops_job() {
    let dir = dirs::create_temp("uploader_requeue_full").await.unwrap();
    let job_b = make_real_job(&dir, "b.log", "contents b").await;
    let (mock, mut started_rx) = MockUploadExecutor::new();
    let (release_tx, release_rx) = oneshot::channel();
    mock.push_step(MockStep::Hang(release_rx));
    mock.push_step(MockStep::Err);
    mock.push_step(MockStep::Err);
    mock.push_step(MockStep::Ok);
    let options = UploaderOptions {
        queue_capacity: 1,
        ..Default::default()
    };
    let (uploader, handle) =
        Uploader::spawn(16, mock.clone(), options, |_: Duration| async {}).unwrap();
    let job_a = make_job("a.log");

    within(uploader.enqueue(job_a.clone())).await.unwrap();
    within(started_rx.recv()).await.unwrap();
    // fill the queue with a fresh (unprunable) job so A's requeue finds it
    // full
    within(uploader.enqueue(job_b.clone())).await.unwrap();
    release_tx.send(scripted_err()).unwrap();
    for _ in 0..3 {
        within(started_rx.recv()).await.unwrap();
    }

    within(uploader.shutdown()).await.unwrap();
    within(handle).await.unwrap();
    // A's failed round could not requeue: A dropped, then B succeeded
    assert_eq!(
        mock.recorded_calls(),
        vec![job_a.clone(), job_a.clone(), job_a, job_b]
    );
}

#[tokio::test]
async fn shutdown_during_backoff_sleep_returns_promptly() {
    let (mock, mut started_rx) = MockUploadExecutor::new();
    mock.push_step(MockStep::Err);
    // a sleep that never completes: shutdown must interrupt the backoff
    let (uploader, handle) = Uploader::spawn(
        16,
        mock.clone(),
        UploaderOptions::default(),
        |_: Duration| std::future::pending::<()>(),
    )
    .unwrap();

    within(uploader.enqueue(make_job("a.log"))).await.unwrap();
    within(started_rx.recv()).await.unwrap();

    timeout(Duration::from_secs(1), uploader.shutdown())
        .await
        .expect("shutdown timed out awaiting the backoff sleep")
        .unwrap();
    within(handle).await.unwrap();
}

#[tokio::test]
async fn enqueue_after_shutdown_returns_send_err() {
    let (mock, _started_rx) = MockUploadExecutor::new();
    let (uploader, handle) = spawn_uploader(mock.clone());

    within(uploader.shutdown()).await.unwrap();
    within(handle).await.unwrap();

    let result = within(uploader.enqueue(make_job("a.log"))).await;
    assert!(
        matches!(result, Err(UploadErr::SendActorMessageErr(_))),
        "expected SendActorMessageErr, got: {result:?}"
    );
}

#[tokio::test]
async fn worker_exits_when_all_handles_dropped() {
    let (mock, _started_rx) = MockUploadExecutor::new();
    let (uploader, handle) = spawn_uploader(mock.clone());

    drop(uploader);
    within(handle).await.unwrap();
}

#[tokio::test]
async fn worker_exits_when_handles_dropped_during_in_flight_upload() {
    let (mock, mut started_rx) = MockUploadExecutor::new();
    let (release_tx, release_rx) = oneshot::channel();
    mock.push_step(MockStep::Hang(release_rx));
    let (uploader, handle) = spawn_uploader(mock.clone());

    within(uploader.enqueue(make_job("a.log"))).await.unwrap();
    within(started_rx.recv()).await.unwrap();

    // the in-flight upload is dropped, not awaited
    drop(uploader);
    within(handle).await.unwrap();
    drop(release_tx);
}

#[tokio::test]
async fn arc_handle_delegates_to_uploader() {
    let (mock, mut started_rx) = MockUploadExecutor::new();
    let (uploader, handle) = spawn_uploader(mock.clone());
    let uploader = Arc::new(uploader);
    let job = make_job("a.log");

    let outcome = within(uploader.enqueue(job.clone())).await.unwrap();
    assert_eq!(outcome, EnqueueOutcome::Enqueued);
    within(started_rx.recv()).await.unwrap();
    assert_eq!(within(uploader.len()).await.unwrap(), 0);

    within(uploader.shutdown()).await.unwrap();
    within(handle).await.unwrap();
    assert_eq!(mock.recorded_calls(), vec![job]);
}

#[tokio::test]
async fn len_reports_queued_jobs() {
    let (mock, mut started_rx) = MockUploadExecutor::new();
    let (release_tx, release_rx) = oneshot::channel();
    mock.push_step(MockStep::Hang(release_rx));
    let (uploader, handle) = spawn_uploader(mock.clone());

    within(uploader.enqueue(make_job("a.log"))).await.unwrap();
    within(started_rx.recv()).await.unwrap();
    within(uploader.enqueue(make_job("b.log"))).await.unwrap();
    within(uploader.enqueue(make_job("c.log"))).await.unwrap();

    // in-flight A is excluded
    assert_eq!(within(uploader.len()).await.unwrap(), 2);

    release_tx.send(Ok(())).unwrap();
    within(uploader.shutdown()).await.unwrap();
    within(handle).await.unwrap();
}
