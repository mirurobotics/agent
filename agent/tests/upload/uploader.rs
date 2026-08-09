// standard crates
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// internal crates
use crate::mocks::upload_executor::{MockStep, MockUploadExecutor};
use miru_agent::filesys::{self, dirs, files, File, WriteOptions};
use miru_agent::upload::errors::ExecutorErr;
use miru_agent::upload::{Job, UploadErr, Uploader, UploaderExt, UploaderOptions};

// external crates
use chrono::{DateTime, Utc};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::timeout;

const TEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Await `fut` with a timeout so sequencing bugs surface as panics, not
/// hanging tests.
async fn timed<T>(fut: impl Future<Output = T>) -> T {
    timeout(TEST_TIMEOUT, fut).await.expect("test timed out")
}

fn make_job(name: &str) -> Job {
    Job {
        file: File::new(format!("/data/{name}")),
        size: 42,
        digest: format!("sha256:{name}"),
        mtime: Utc::now(),
        first_observed_at: Utc::now(),
        last_observed_at: Utc::now(),
        file_rule_id: "rule_1".to_string(),
        deployment_id: "dpl_1".to_string(),
        retention: None,
    }
}

fn scripted_err() -> Result<(), UploadErr> {
    Err(UploadErr::ExecutorErr(ExecutorErr {
        source: Box::new(std::io::Error::other("scripted failure")),
        terminal_status: None,
        trace: miru_agent::trace!(),
    }))
}

/// Build a job backed by a real temp file, so queue staleness checks see it
/// as fresh.
async fn make_real_job(dir: &filesys::Dir, name: &str, contents: &str) -> Job {
    let file = dir.file(name);
    files::write_string(&file, contents, WriteOptions::OVERWRITE_ATOMIC)
        .await
        .unwrap();
    Job {
        file: file.clone(),
        size: files::size(&file).await.unwrap(),
        digest: files::hash(&file).await.unwrap(),
        mtime: DateTime::<Utc>::from(files::last_modified(&file).await.unwrap()),
        first_observed_at: Utc::now(),
        last_observed_at: Utc::now(),
        file_rule_id: "rule_1".to_string(),
        deployment_id: "dpl_1".to_string(),
        retention: None,
    }
}

/// Spawn an uploader with default options and a no-op sleep.
fn spawn_uploader(mock: Arc<MockUploadExecutor>) -> (Uploader, JoinHandle<()>) {
    Uploader::spawn(
        16,
        mock,
        UploaderOptions::default(),
        None,
        |_: Duration| async {},
    )
    .unwrap()
}

#[test]
fn attempt_deadline_formula() {
    // pin the inputs so the assertions are independent of production defaults
    let options = UploaderOptions {
        attempt_timeout_floor: Duration::from_secs(10),
        attempt_timeout_bytes_per_sec: 100,
        ..UploaderOptions::default()
    };

    // size 0 pays the floor only
    assert_eq!(options.attempt_deadline(0), Duration::from_secs(10));
    // a partial second of transfer time rounds up
    assert_eq!(options.attempt_deadline(1), Duration::from_secs(11));
    // an exact multiple adds size / bytes-per-second seconds
    assert_eq!(options.attempt_deadline(500), Duration::from_secs(15));

    // production defaults: 120s floor plus one second per 64 KiB
    let defaults = UploaderOptions::default();
    assert_eq!(defaults.attempt_deadline(0), Duration::from_secs(120));
    assert_eq!(
        defaults.attempt_deadline(64 * 1024),
        Duration::from_secs(121)
    );

    // a zero throughput assumption clamps to 1 byte/sec instead of panicking
    let zero_bps = UploaderOptions {
        attempt_timeout_floor: Duration::from_secs(10),
        attempt_timeout_bytes_per_sec: 0,
        ..UploaderOptions::default()
    };
    assert_eq!(zero_bps.attempt_deadline(1_000), Duration::from_secs(1_010));
}

#[tokio::test]
async fn processes_enqueued_job() {
    let (mock, mut started_rx) = MockUploadExecutor::new();
    let (uploader, handle) = spawn_uploader(mock.clone());
    let job = make_job("a.log");

    timed(uploader.enqueue(job.clone())).await.unwrap();
    timed(started_rx.recv()).await.unwrap();

    assert_eq!(mock.recorded_calls(), vec![job]);
    assert_eq!(timed(uploader.len()).await.unwrap(), 0);

    timed(uploader.shutdown()).await.unwrap();
    timed(handle).await.unwrap();
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

    timed(uploader.enqueue(job_a.clone())).await.unwrap();
    timed(started_rx.recv()).await.unwrap();
    // B is queued while A is in flight, so it lands ahead of A's requeue slot
    timed(uploader.enqueue(job_b.clone())).await.unwrap();
    release_tx.send(scripted_err()).unwrap();
    for _ in 0..4 {
        timed(started_rx.recv()).await.unwrap();
    }

    timed(uploader.shutdown()).await.unwrap();
    timed(handle).await.unwrap();
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

    timed(uploader.enqueue(job_a.clone())).await.unwrap();
    // three rounds of three attempts each
    for _ in 0..9 {
        timed(started_rx.recv()).await.unwrap();
    }

    // A was dropped at the cap with the actor still healthy: B processes next
    timed(uploader.enqueue(job_b.clone())).await.unwrap();
    timed(started_rx.recv()).await.unwrap();

    timed(uploader.shutdown()).await.unwrap();
    timed(handle).await.unwrap();
    let mut expected = vec![job_a; 9];
    expected.push(job_b);
    assert_eq!(mock.recorded_calls(), expected);
}

#[tokio::test]
async fn terminal_failure_drops_job_without_requeue() {
    let (mock, mut started_rx) = MockUploadExecutor::new();
    mock.push_step(MockStep::TerminalErr);
    mock.push_step(MockStep::Ok);
    let (uploader, handle) = spawn_uploader(mock.clone());
    let job_a = make_job("a.log");
    let job_b = make_job("b.log");

    timed(uploader.enqueue(job_a.clone())).await.unwrap();
    timed(started_rx.recv()).await.unwrap();
    timed(uploader.enqueue(job_b.clone())).await.unwrap();
    timed(started_rx.recv()).await.unwrap();

    // A was dropped on its first attempt: nothing queued for a requeue round
    assert_eq!(timed(uploader.len()).await.unwrap(), 0);

    timed(uploader.shutdown()).await.unwrap();
    timed(handle).await.unwrap();
    // exactly one attempt for A (no in-place retries), then B: the worker survived
    assert_eq!(mock.recorded_calls(), vec![job_a, job_b]);
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
    // pin the backoff so the assertion is independent of production defaults
    let options = UploaderOptions {
        backoff: miru_agent::cooldown::Backoff {
            base_secs: 1,
            growth_factor: 2,
            max_secs: 30,
        },
        ..UploaderOptions::default()
    };
    let (uploader, handle) = Uploader::spawn(16, mock.clone(), options, None, sleep_fn).unwrap();

    timed(uploader.enqueue(make_job("a.log"))).await.unwrap();
    for _ in 0..7 {
        timed(started_rx.recv()).await.unwrap();
    }

    // in-place sleeps only (exponent = this round's attempts - 1); none around
    // the two requeues, and the backoff resets with each round
    let expected = vec![
        Duration::from_secs(1),
        Duration::from_secs(2),
        Duration::from_secs(1),
        Duration::from_secs(2),
    ];
    assert_eq!(*sleeps.lock().unwrap(), expected);

    timed(uploader.shutdown()).await.unwrap();
    timed(handle).await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn hung_attempt_times_out_and_is_retried() {
    let (mock, mut started_rx) = MockUploadExecutor::new();
    let (release_tx, release_rx) = oneshot::channel();

    // the hang is never released: only the attempt deadline can end it
    mock.push_step(MockStep::Hang(release_rx));
    mock.push_step(MockStep::Ok);
    let sleeps: Arc<Mutex<Vec<Duration>>> = Arc::new(Mutex::new(Vec::new()));
    let recorded = sleeps.clone();

    // instant sleeps, so the attempt deadline is the only pending timer and
    // the paused clock auto-advances straight to it
    let sleep_fn = move |duration: Duration| {
        recorded.lock().unwrap().push(duration);
        async {}
    };

    // pin the deadline inputs: make_job's 42 bytes yield a 2s deadline,
    // safely under timed()'s 5s guard
    let options = UploaderOptions {
        attempt_timeout_floor: Duration::from_secs(1),
        attempt_timeout_bytes_per_sec: 64 * 1024,
        ..UploaderOptions::default()
    };
    let (uploader, handle) = Uploader::spawn(16, mock.clone(), options, None, sleep_fn).unwrap();
    let job = make_job("a.log");

    timed(uploader.enqueue(job.clone())).await.unwrap();
    // first attempt starts and hangs on the never-released oneshot
    timed(started_rx.recv()).await.unwrap();
    // the deadline fires in virtual time and the retry attempt starts
    timed(started_rx.recv()).await.unwrap();

    // the same job was attempted twice: the timeout was treated as a
    // retryable failure, taking the normal in-place backoff path
    assert_eq!(mock.recorded_calls(), vec![job.clone(), job]);
    assert!(!sleeps.lock().unwrap().is_empty());

    timed(uploader.shutdown()).await.unwrap();
    timed(handle).await.unwrap();
    drop(release_tx);
}

#[tokio::test]
async fn shutdown_during_in_flight_upload_returns_promptly() {
    let (mock, mut started_rx) = MockUploadExecutor::new();
    let (release_tx, release_rx) = oneshot::channel();
    mock.push_step(MockStep::Hang(release_rx));
    let (uploader, handle) = spawn_uploader(mock.clone());

    timed(uploader.enqueue(make_job("a.log"))).await.unwrap();
    timed(started_rx.recv()).await.unwrap();

    // the hang is never released: shutdown must drop the in-flight future
    // rather than await it
    timeout(Duration::from_secs(1), uploader.shutdown())
        .await
        .expect("shutdown timed out awaiting the in-flight upload")
        .unwrap();
    timed(handle).await.unwrap();
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
        Uploader::spawn(16, mock.clone(), options, None, |_: Duration| async {}).unwrap();
    let job_a = make_job("a.log");

    timed(uploader.enqueue(job_a.clone())).await.unwrap();
    timed(started_rx.recv()).await.unwrap();
    // fill the queue with a fresh (unprunable) job so A's requeue finds it
    // full
    timed(uploader.enqueue(job_b.clone())).await.unwrap();
    release_tx.send(scripted_err()).unwrap();
    for _ in 0..3 {
        timed(started_rx.recv()).await.unwrap();
    }

    timed(uploader.shutdown()).await.unwrap();
    timed(handle).await.unwrap();
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
        None,
        |_: Duration| std::future::pending::<()>(),
    )
    .unwrap();

    timed(uploader.enqueue(make_job("a.log"))).await.unwrap();
    timed(started_rx.recv()).await.unwrap();

    timeout(Duration::from_secs(1), uploader.shutdown())
        .await
        .expect("shutdown timed out awaiting the backoff sleep")
        .unwrap();
    timed(handle).await.unwrap();
}

#[tokio::test]
async fn enqueue_after_shutdown_returns_send_err() {
    let (mock, _started_rx) = MockUploadExecutor::new();
    let (uploader, handle) = spawn_uploader(mock.clone());

    timed(uploader.shutdown()).await.unwrap();
    timed(handle).await.unwrap();

    let result = timed(uploader.enqueue(make_job("a.log"))).await;
    assert!(
        matches!(result, Err(UploadErr::SendActorMessageErr(_))),
        "expected SendActorMessageErr, got: {result:?}"
    );
    assert_eq!(result.unwrap_err().terminal_status(), None);
}

#[tokio::test]
async fn len_after_shutdown_returns_send_err() {
    let (mock, _started_rx) = MockUploadExecutor::new();
    let (uploader, handle) = spawn_uploader(mock.clone());

    timed(uploader.shutdown()).await.unwrap();
    timed(handle).await.unwrap();

    let result = timed(uploader.len()).await;
    assert!(
        matches!(result, Err(UploadErr::SendActorMessageErr(_))),
        "expected SendActorMessageErr, got: {result:?}"
    );
}

#[tokio::test]
async fn shutdown_after_worker_exit_returns_send_err() {
    let (mock, _started_rx) = MockUploadExecutor::new();
    let (uploader, handle) = spawn_uploader(mock.clone());

    timed(uploader.shutdown()).await.unwrap();
    timed(handle).await.unwrap();

    // the worker is gone; a second shutdown can't reach it
    let result = timed(uploader.shutdown()).await;
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
    timed(handle).await.unwrap();
}

#[tokio::test]
async fn worker_exits_when_handles_dropped_during_in_flight_upload() {
    let (mock, mut started_rx) = MockUploadExecutor::new();
    let (release_tx, release_rx) = oneshot::channel();
    mock.push_step(MockStep::Hang(release_rx));
    let (uploader, handle) = spawn_uploader(mock.clone());

    timed(uploader.enqueue(make_job("a.log"))).await.unwrap();
    timed(started_rx.recv()).await.unwrap();

    // the in-flight upload is dropped, not awaited
    drop(uploader);
    timed(handle).await.unwrap();
    drop(release_tx);
}

#[tokio::test]
async fn arc_handle_delegates_to_uploader() {
    let (mock, mut started_rx) = MockUploadExecutor::new();
    let (uploader, handle) = spawn_uploader(mock.clone());
    let uploader = Arc::new(uploader);
    let job = make_job("a.log");

    timed(uploader.enqueue(job.clone())).await.unwrap();
    timed(started_rx.recv()).await.unwrap();
    assert_eq!(timed(uploader.len()).await.unwrap(), 0);

    timed(uploader.shutdown()).await.unwrap();
    timed(handle).await.unwrap();
    assert_eq!(mock.recorded_calls(), vec![job]);
}

#[tokio::test]
async fn len_reports_queued_jobs() {
    let (mock, mut started_rx) = MockUploadExecutor::new();
    let (release_tx, release_rx) = oneshot::channel();
    mock.push_step(MockStep::Hang(release_rx));
    let (uploader, handle) = spawn_uploader(mock.clone());

    timed(uploader.enqueue(make_job("a.log"))).await.unwrap();
    timed(started_rx.recv()).await.unwrap();
    timed(uploader.enqueue(make_job("b.log"))).await.unwrap();
    timed(uploader.enqueue(make_job("c.log"))).await.unwrap();

    // in-flight A is excluded
    assert_eq!(timed(uploader.len()).await.unwrap(), 2);

    release_tx.send(Ok(())).unwrap();
    timed(uploader.shutdown()).await.unwrap();
    timed(handle).await.unwrap();
}
