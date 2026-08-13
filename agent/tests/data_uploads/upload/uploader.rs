// standard crates
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// internal crates
use crate::mocks::upload_executor::{MockStep, MockUploadExecutor};
use miru_agent::data_uploads::upload::errors::ExecutorErr;
use miru_agent::data_uploads::upload::{
    Job, QueueSnapshot, QueueSnapshotFile, UploadErr, Uploader, UploaderExt, UploaderOptions,
};
use miru_agent::errors::Error;
use miru_agent::filesys::{dirs, File};

// external crates
use chrono::{TimeDelta, Utc};
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
        is_terminal: false,
        is_network_conn_err: false,
        trace: miru_agent::trace!(),
    }))
}

fn scripted_network_err() -> Result<(), UploadErr> {
    Err(UploadErr::ExecutorErr(ExecutorErr {
        source: Box::new(std::io::Error::other("scripted network failure")),
        is_terminal: false,
        is_network_conn_err: true,
        trace: miru_agent::trace!(),
    }))
}

/// Spawn an uploader with default options and a no-op sleep.
///
/// Safe ONLY for tests whose script never leaves a stamped (backoff) entry
/// waiting in the queue: a no-op sleep plus a non-advancing clock busy-loops
/// the current-thread runtime and hangs the test binary. Use
/// [`spawn_with_test_clock`] or [`spawn_frozen`] for anything backoff-shaped.
fn spawn_uploader(mock: Arc<MockUploadExecutor>) -> (Uploader, JoinHandle<()>) {
    Uploader::spawn(
        16,
        mock,
        UploaderOptions::default(),
        None,
        |_: Duration| async {},
        Utc::now,
    )
    .unwrap()
}

/// Spawn an uploader over a shared test clock: `sleep_fn` records each
/// requested duration into the returned log and advances the clock by it, so
/// backoff waits complete instantly and stamped entries become eligible
/// deterministically.
fn spawn_with_test_clock(
    mock: Arc<MockUploadExecutor>,
    options: UploaderOptions,
) -> (Uploader, JoinHandle<()>, Arc<Mutex<Vec<Duration>>>) {
    let clock = Arc::new(Mutex::new(Utc::now()));
    let sleeps: Arc<Mutex<Vec<Duration>>> = Arc::new(Mutex::new(Vec::new()));
    let now_clock = clock.clone();
    let now_fn = move || *now_clock.lock().unwrap();
    let recorded = sleeps.clone();
    let sleep_fn = move |duration: Duration| {
        recorded.lock().unwrap().push(duration);
        *clock.lock().unwrap() += TimeDelta::from_std(duration).unwrap();
        async {}
    };
    let (uploader, handle) = Uploader::spawn(16, mock, options, None, sleep_fn, now_fn).unwrap();
    (uploader, handle, sleeps)
}

/// Spawn an uploader over a clock frozen at spawn time and a sleep that never
/// completes: stamped (backoff) entries never become eligible, so the worker
/// parks in its idle wait until a command arrives.
fn spawn_frozen(
    mock: Arc<MockUploadExecutor>,
    options: UploaderOptions,
) -> (Uploader, JoinHandle<()>) {
    let epoch = Utc::now();
    Uploader::spawn(
        16,
        mock,
        options,
        None,
        |_: Duration| std::future::pending::<()>(),
        move || epoch,
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
async fn failed_upload_moves_on_to_next_job() {
    let (mock, mut started_rx) = MockUploadExecutor::new();
    let (release_tx, release_rx) = oneshot::channel();
    mock.push_step(MockStep::Hang(release_rx));
    mock.push_step(MockStep::Ok);
    mock.push_step(MockStep::Ok);
    let (uploader, handle, _sleeps) =
        spawn_with_test_clock(mock.clone(), UploaderOptions::default());
    let job_a = make_job("a.log");
    let job_b = make_job("b.log");

    timed(uploader.enqueue(job_a.clone())).await.unwrap();
    timed(started_rx.recv()).await.unwrap();
    // B is queued while A is in flight, so it lands ahead of A's requeue slot
    timed(uploader.enqueue(job_b.clone())).await.unwrap();
    release_tx.send(scripted_err()).unwrap();
    for _ in 0..2 {
        timed(started_rx.recv()).await.unwrap();
    }

    timed(uploader.shutdown()).await.unwrap();
    timed(handle).await.unwrap();
    // A's failure requeues it at the tail: B runs next, then A's retry succeeds
    assert_eq!(mock.recorded_calls(), vec![job_a.clone(), job_b, job_a]);
}

#[tokio::test]
async fn attempt_cap_drops_job() {
    let (mock, mut started_rx) = MockUploadExecutor::new();
    for _ in 0..3 {
        mock.push_step(MockStep::Err);
    }
    mock.push_step(MockStep::Ok);
    let options = UploaderOptions {
        attempts: 3,
        ..UploaderOptions::default()
    };
    let (uploader, handle, _sleeps) = spawn_with_test_clock(mock.clone(), options);
    let job_a = make_job("a.log");
    let job_b = make_job("b.log");

    timed(uploader.enqueue(job_a.clone())).await.unwrap();
    // the script is positional: enqueue B only after all three of A's attempts
    // have started, or B would pop during A's backoff and consume A's Err step
    for _ in 0..3 {
        timed(started_rx.recv()).await.unwrap();
    }

    // A was dropped at the cap with the actor still healthy: B processes next
    timed(uploader.enqueue(job_b.clone())).await.unwrap();
    timed(started_rx.recv()).await.unwrap();
    assert_eq!(timed(uploader.len()).await.unwrap(), 0);

    timed(uploader.shutdown()).await.unwrap();
    timed(handle).await.unwrap();
    let expected = vec![job_a.clone(), job_a.clone(), job_a, job_b];
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
    for _ in 0..4 {
        mock.push_step(MockStep::Err);
    }
    mock.push_step(MockStep::Ok);
    // pin the backoff so the assertion is independent of production defaults
    let options = UploaderOptions {
        attempts: 5,
        backoff: miru_agent::cooldown::Backoff {
            base_secs: 1,
            growth_factor: 2,
            max_secs: 4,
        },
        ..UploaderOptions::default()
    };
    let (uploader, handle, sleeps) = spawn_with_test_clock(mock.clone(), options);

    timed(uploader.enqueue(make_job("a.log"))).await.unwrap();
    for _ in 0..5 {
        timed(started_rx.recv()).await.unwrap();
    }

    timed(uploader.shutdown()).await.unwrap();
    timed(handle).await.unwrap();
    // the exponent is the lifetime attempt count minus one, capped at max_secs
    let expected = vec![
        Duration::from_secs(1),
        Duration::from_secs(2),
        Duration::from_secs(4),
        Duration::from_secs(4),
    ];
    assert_eq!(*sleeps.lock().unwrap(), expected);
}

#[tokio::test(start_paused = true)]
async fn hung_attempt_times_out_and_is_retried() {
    let (mock, mut started_rx) = MockUploadExecutor::new();
    let (release_tx, release_rx) = oneshot::channel();

    // the hang is never released: only the attempt deadline can end it
    mock.push_step(MockStep::Hang(release_rx));
    mock.push_step(MockStep::Ok);

    // pin the deadline inputs: make_job's 42 bytes yield a 2s deadline,
    // safely under timed()'s 5s guard
    let options = UploaderOptions {
        attempt_timeout_floor: Duration::from_secs(1),
        attempt_timeout_bytes_per_sec: 64 * 1024,
        ..UploaderOptions::default()
    };
    // the test clock's instant sleeps leave the attempt deadline as the only
    // pending timer, so the paused tokio clock auto-advances straight to it;
    // the chrono test clock then advances past the backoff stamp so the
    // requeued job becomes eligible deterministically
    let (uploader, handle, sleeps) = spawn_with_test_clock(mock.clone(), options);
    let job = make_job("a.log");

    timed(uploader.enqueue(job.clone())).await.unwrap();
    // first attempt starts and hangs on the never-released oneshot
    timed(started_rx.recv()).await.unwrap();
    // the deadline fires in virtual time; the job is stamped with a backoff
    // deadline, requeued, and retried once the test clock reaches the stamp
    timed(started_rx.recv()).await.unwrap();

    // the same job was attempted twice: the timeout was treated as a
    // retryable failure, taking the normal backoff/requeue path
    assert_eq!(mock.recorded_calls(), vec![job.clone(), job]);
    assert!(!sleeps.lock().unwrap().is_empty());

    timed(uploader.shutdown()).await.unwrap();
    timed(handle).await.unwrap();
    drop(release_tx);
}

#[tokio::test]
async fn network_failures_never_drop_job() {
    let (mock, mut started_rx) = MockUploadExecutor::new();
    for _ in 0..6 {
        mock.push_step(MockStep::NetworkErr);
    }
    mock.push_step(MockStep::Ok);
    // a budget the network failures would blow through if they counted
    let options = UploaderOptions {
        attempts: 3,
        ..UploaderOptions::default()
    };
    let (uploader, handle, _sleeps) = spawn_with_test_clock(mock.clone(), options);
    let job = make_job("a.log");

    timed(uploader.enqueue(job.clone())).await.unwrap();
    for _ in 0..7 {
        timed(started_rx.recv()).await.unwrap();
    }
    assert_eq!(timed(uploader.len()).await.unwrap(), 0);

    timed(uploader.shutdown()).await.unwrap();
    timed(handle).await.unwrap();
    assert_eq!(mock.recorded_calls(), vec![job; 7]);
}

#[tokio::test]
async fn network_failure_uses_flat_cooldown() {
    let (mock, mut started_rx) = MockUploadExecutor::new();
    mock.push_step(MockStep::NetworkErr);
    mock.push_step(MockStep::NetworkErr);
    mock.push_step(MockStep::Ok);
    // pin the backoff so the assertion is independent of production defaults
    let options = UploaderOptions {
        backoff: miru_agent::cooldown::Backoff {
            base_secs: 1,
            growth_factor: 2,
            max_secs: 30,
        },
        ..UploaderOptions::default()
    };
    let (uploader, handle, sleeps) = spawn_with_test_clock(mock.clone(), options);

    timed(uploader.enqueue(make_job("a.log"))).await.unwrap();
    for _ in 0..3 {
        timed(started_rx.recv()).await.unwrap();
    }

    timed(uploader.shutdown()).await.unwrap();
    timed(handle).await.unwrap();
    // the flat base period after each network failure; the growing backoff
    // would be [1s, 2s]
    assert_eq!(
        *sleeps.lock().unwrap(),
        vec![Duration::from_secs(1), Duration::from_secs(1)]
    );
}

#[tokio::test]
async fn network_failure_requeues_at_tail() {
    let (mock, mut started_rx) = MockUploadExecutor::new();
    let (release_tx, release_rx) = oneshot::channel();
    mock.push_step(MockStep::Hang(release_rx));
    mock.push_step(MockStep::Ok);
    mock.push_step(MockStep::Ok);
    let (uploader, handle, _sleeps) =
        spawn_with_test_clock(mock.clone(), UploaderOptions::default());
    let job_a = make_job("a.log");
    let job_b = make_job("b.log");

    timed(uploader.enqueue(job_a.clone())).await.unwrap();
    timed(started_rx.recv()).await.unwrap();
    // B is queued while A is in flight, so it lands ahead of A's requeue slot
    timed(uploader.enqueue(job_b.clone())).await.unwrap();
    release_tx.send(scripted_network_err()).unwrap();
    for _ in 0..2 {
        timed(started_rx.recv()).await.unwrap();
    }

    timed(uploader.shutdown()).await.unwrap();
    timed(handle).await.unwrap();
    // A's network failure stamps its flat cooldown and requeues at the tail:
    // B, eligible now, runs before A's retry
    assert_eq!(mock.recorded_calls(), vec![job_a.clone(), job_b, job_a]);
}

#[tokio::test]
async fn network_failures_do_not_consume_attempt_budget() {
    let (mock, mut started_rx) = MockUploadExecutor::new();
    // the whole 3-attempt budget in non-network failures, interleaved with two
    // network failures that must not count toward it: A is dropped on the
    // third non-network failure (call 5), not before
    for step in [
        MockStep::Err,
        MockStep::NetworkErr,
        MockStep::Err,
        MockStep::NetworkErr,
        MockStep::Err,
    ] {
        mock.push_step(step);
    }
    mock.push_step(MockStep::Ok);
    let options = UploaderOptions {
        attempts: 3,
        ..UploaderOptions::default()
    };
    let (uploader, handle, _sleeps) = spawn_with_test_clock(mock.clone(), options);
    let job_a = make_job("a.log");
    let job_b = make_job("b.log");

    timed(uploader.enqueue(job_a.clone())).await.unwrap();
    for _ in 0..5 {
        timed(started_rx.recv()).await.unwrap();
    }

    // A was dropped at the cap with the actor still healthy: B processes next
    timed(uploader.enqueue(job_b.clone())).await.unwrap();
    timed(started_rx.recv()).await.unwrap();

    timed(uploader.shutdown()).await.unwrap();
    timed(handle).await.unwrap();
    let mut expected = vec![job_a; 5];
    expected.push(job_b);
    assert_eq!(mock.recorded_calls(), expected);
}

#[tokio::test]
async fn network_failure_past_max_job_age_drops_job() {
    let (mock, mut started_rx) = MockUploadExecutor::new();
    mock.push_step(MockStep::NetworkErr);
    mock.push_step(MockStep::Ok);
    let (uploader, handle, _sleeps) =
        spawn_with_test_clock(mock.clone(), UploaderOptions::default());
    // first observed past the backstop, so the network exemption no longer
    // applies — the only way a misclassified permanent failure ever exits
    let mut job_a = make_job("a.log");
    job_a.first_observed_at = Utc::now() - TimeDelta::days(8);
    let job_b = make_job("b.log");

    timed(uploader.enqueue(job_a.clone())).await.unwrap();
    timed(started_rx.recv()).await.unwrap();

    // A was dropped on its single network failure, not requeued: B runs next
    timed(uploader.enqueue(job_b.clone())).await.unwrap();
    timed(started_rx.recv()).await.unwrap();
    assert_eq!(timed(uploader.len()).await.unwrap(), 0);

    timed(uploader.shutdown()).await.unwrap();
    timed(handle).await.unwrap();
    assert_eq!(mock.recorded_calls(), vec![job_a, job_b]);
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

/// An in-flight job keeps its slot: it stays queued until it is confirmed,
/// so it is still counted against capacity. A requeue can therefore never
/// push the queue past capacity the way it could when the pop freed the slot.
#[tokio::test]
async fn in_flight_job_holds_its_capacity_slot() {
    let (mock, mut started_rx) = MockUploadExecutor::new();
    let (release_tx, release_rx) = oneshot::channel();
    mock.push_step(MockStep::Hang(release_rx));
    mock.push_step(MockStep::Ok);
    let options = UploaderOptions {
        queue_capacity: 1,
        ..Default::default()
    };
    let (uploader, handle, _sleeps) = spawn_with_test_clock(mock.clone(), options);
    let job_a = make_job("a.log");

    timed(uploader.enqueue(job_a.clone())).await.unwrap();
    timed(started_rx.recv()).await.unwrap();
    // A still occupies the only slot, so B is rejected rather than admitted
    let err = timed(uploader.enqueue(make_job("b.log")))
        .await
        .unwrap_err();
    assert!(
        matches!(err, UploadErr::QueueFullErr(_)),
        "expected QueueFullErr, got: {err:?}"
    );

    // A's failure requeues it in place: still one job, retried after backoff
    release_tx.send(scripted_err()).unwrap();
    timed(started_rx.recv()).await.unwrap();

    timed(uploader.shutdown()).await.unwrap();
    timed(handle).await.unwrap();
    assert_eq!(mock.recorded_calls(), vec![job_a.clone(), job_a]);
}

#[tokio::test]
async fn shutdown_during_backoff_sleep_returns_promptly() {
    let (mock, mut started_rx) = MockUploadExecutor::new();
    mock.push_step(MockStep::Err);
    // a sleep that never completes: shutdown must interrupt the idle wait
    // over the requeued entry's backoff deadline
    let (uploader, handle) = Uploader::spawn(
        16,
        mock.clone(),
        UploaderOptions::default(),
        None,
        |_: Duration| std::future::pending::<()>(),
        Utc::now,
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
async fn waiting_job_is_skipped_and_enqueue_wakes_idle_wait() {
    let (mock, mut started_rx) = MockUploadExecutor::new();
    mock.push_step(MockStep::Err);
    mock.push_step(MockStep::Ok);
    let (uploader, handle) = spawn_frozen(mock.clone(), UploaderOptions::default());
    let job_a = make_job("a.log");
    let job_b = make_job("b.log");

    timed(uploader.enqueue(job_a.clone())).await.unwrap();
    timed(started_rx.recv()).await.unwrap();
    // A is stamped with a deadline the frozen clock never reaches; the enqueue
    // must wake the idle wait so B runs while A keeps waiting
    timed(uploader.enqueue(job_b.clone())).await.unwrap();
    timed(started_rx.recv()).await.unwrap();
    assert_eq!(timed(uploader.len()).await.unwrap(), 1);

    timeout(Duration::from_secs(1), uploader.shutdown())
        .await
        .expect("shutdown timed out awaiting the idle wait")
        .unwrap();
    timed(handle).await.unwrap();
    assert_eq!(mock.recorded_calls(), vec![job_a, job_b]);
}

#[tokio::test]
async fn worker_exits_when_handles_dropped_during_idle_wait() {
    let (mock, mut started_rx) = MockUploadExecutor::new();
    mock.push_step(MockStep::Err);
    let (uploader, handle) = spawn_frozen(mock.clone(), UploaderOptions::default());

    timed(uploader.enqueue(make_job("a.log"))).await.unwrap();
    timed(started_rx.recv()).await.unwrap();

    // the worker is parked in its idle wait over A's backoff deadline;
    // dropping the last handle must end it
    drop(uploader);
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
    assert!(!result.unwrap_err().is_terminal());
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
async fn command_pending_at_shutdown_returns_receive_err() {
    let (mock, _started_rx) = MockUploadExecutor::new();
    let (uploader, handle) = spawn_uploader(mock.clone());

    // queue a shutdown command, then drop its response channel: the worker
    // must still shut down even though its acknowledgement has nowhere to go
    let mut shutdown_fut = Box::pin(uploader.shutdown());
    std::future::poll_fn(|cx| {
        assert!(shutdown_fut.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    drop(shutdown_fut);

    // on the current-thread runtime the worker only wakes once this await
    // yields, so the len command is already queued behind the shutdown; the
    // worker's exit drops it unanswered
    let result = timed(uploader.len()).await;
    assert!(
        matches!(result, Err(UploadErr::ReceiveActorMessageErr(_))),
        "expected ReceiveActorMessageErr, got: {result:?}"
    );
    timed(handle).await.unwrap();
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

    // the in-flight job stays queued until it is confirmed, so it counts
    assert_eq!(timed(uploader.len()).await.unwrap(), 3);

    release_tx.send(Ok(())).unwrap();
    timed(uploader.shutdown()).await.unwrap();
    timed(handle).await.unwrap();
}

mod durability {
    use super::*;

    /// A queue persisted to `path`, so a test can inspect what is on disk
    /// while an attempt is still running.
    async fn open(path: &File) -> QueueSnapshotFile {
        QueueSnapshotFile::new_with_default(path.clone(), QueueSnapshot::default())
            .await
            .unwrap()
    }

    fn spawn_persisted(
        mock: Arc<MockUploadExecutor>,
        snapshot: QueueSnapshotFile,
    ) -> (Uploader, JoinHandle<()>) {
        Uploader::spawn(
            16,
            mock,
            UploaderOptions::default(),
            Some(snapshot),
            |_: Duration| async {},
            Utc::now,
        )
        .unwrap()
    }

    /// The digests currently on disk, read through a fresh handle.
    async fn on_disk(path: &File) -> Vec<String> {
        open(path)
            .await
            .read()
            .entries
            .iter()
            .map(|entry| entry.job.digest.clone())
            .collect()
    }

    /// Wait until the queue is empty. Releasing a hung attempt and shutting
    /// down immediately is a race — `run_until_shutdown`'s select is not
    /// biased, so it may take the shutdown before the completed upload —
    /// and this makes the completion the thing we wait on.
    async fn await_drained(uploader: &Uploader) {
        timed(async {
            while uploader.len().await.unwrap() > 0 {
                tokio::task::yield_now().await;
            }
        })
        .await;
    }

    /// The core guarantee: a job being uploaded is still on disk, so a crash
    /// mid-transfer leaves it to be retried rather than losing it.
    #[tokio::test]
    async fn job_stays_on_disk_during_attempt() {
        let dir = dirs::temp("uploader_durability").unwrap();
        let path = dir.to_dir().file("upload_queue.json");
        let (mock, mut started_rx) = MockUploadExecutor::new();
        let (release_tx, release_rx) = oneshot::channel();
        mock.push_step(MockStep::Hang(release_rx));
        let (uploader, handle) = spawn_persisted(mock.clone(), open(&path).await);

        timed(uploader.enqueue(make_job("a.log"))).await.unwrap();
        timed(started_rx.recv()).await.unwrap();

        assert_eq!(on_disk(&path).await, vec!["sha256:a.log".to_string()]);

        release_tx.send(Ok(())).unwrap();
        timed(uploader.shutdown()).await.unwrap();
        timed(handle).await.unwrap();
    }

    #[tokio::test]
    async fn confirmed_job_leaves_disk() {
        let dir = dirs::temp("uploader_durability").unwrap();
        let path = dir.to_dir().file("upload_queue.json");
        let (mock, mut started_rx) = MockUploadExecutor::new();
        let (release_tx, release_rx) = oneshot::channel();
        mock.push_step(MockStep::Hang(release_rx));
        let (uploader, handle) = spawn_persisted(mock.clone(), open(&path).await);

        timed(uploader.enqueue(make_job("a.log"))).await.unwrap();
        timed(started_rx.recv()).await.unwrap();
        release_tx.send(Ok(())).unwrap();
        await_drained(&uploader).await;

        assert!(on_disk(&path).await.is_empty());

        timed(uploader.shutdown()).await.unwrap();
        timed(handle).await.unwrap();
    }

    #[tokio::test]
    async fn shutdown_mid_attempt_leaves_job_queued() {
        let dir = dirs::temp("uploader_durability").unwrap();
        let path = dir.to_dir().file("upload_queue.json");
        let (mock, mut started_rx) = MockUploadExecutor::new();
        let (_release_tx, release_rx) = oneshot::channel();
        mock.push_step(MockStep::Hang(release_rx));
        let (uploader, handle) = spawn_persisted(mock.clone(), open(&path).await);

        timed(uploader.enqueue(make_job("a.log"))).await.unwrap();
        timed(started_rx.recv()).await.unwrap();
        // shut down while the attempt is still hanging
        timed(uploader.shutdown()).await.unwrap();
        timed(handle).await.unwrap();

        assert_eq!(on_disk(&path).await, vec!["sha256:a.log".to_string()]);
    }

    #[tokio::test]
    async fn restart_resumes_the_interrupted_job() {
        let dir = dirs::temp("uploader_durability").unwrap();
        let path = dir.to_dir().file("upload_queue.json");
        let job = make_job("a.log");

        let (mock, mut started_rx) = MockUploadExecutor::new();
        let (_release_tx, release_rx) = oneshot::channel();
        mock.push_step(MockStep::Hang(release_rx));
        let (uploader, handle) = spawn_persisted(mock.clone(), open(&path).await);
        timed(uploader.enqueue(job.clone())).await.unwrap();
        timed(started_rx.recv()).await.unwrap();
        timed(uploader.shutdown()).await.unwrap();
        timed(handle).await.unwrap();

        // a second process over the same snapshot file picks the job back up
        let (mock2, mut started2_rx) = MockUploadExecutor::new();
        mock2.push_step(MockStep::Ok);
        let (uploader2, handle2) = spawn_persisted(mock2.clone(), open(&path).await);
        timed(started2_rx.recv()).await.unwrap();
        await_drained(&uploader2).await;

        assert_eq!(mock2.recorded_calls(), vec![job]);
        assert!(on_disk(&path).await.is_empty());

        timed(uploader2.shutdown()).await.unwrap();
        timed(handle2).await.unwrap();
    }

    #[tokio::test]
    async fn terminal_failure_removes_job_from_disk() {
        let dir = dirs::temp("uploader_durability").unwrap();
        let path = dir.to_dir().file("upload_queue.json");
        let (mock, mut started_rx) = MockUploadExecutor::new();
        mock.push_step(MockStep::TerminalErr);
        let (uploader, handle) = spawn_persisted(mock.clone(), open(&path).await);

        timed(uploader.enqueue(make_job("a.log"))).await.unwrap();
        timed(started_rx.recv()).await.unwrap();
        await_drained(&uploader).await;

        assert!(on_disk(&path).await.is_empty());

        timed(uploader.shutdown()).await.unwrap();
        timed(handle).await.unwrap();
    }
}

#[test]
fn default_options_retry_for_hours() {
    let options = UploaderOptions::default();
    assert_eq!(options.attempts, 30);
    assert_eq!(options.backoff.max_secs, 3600);
}
