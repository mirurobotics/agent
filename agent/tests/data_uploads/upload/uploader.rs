// standard crates
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// internal crates
use crate::mocks::{
    deleter::{MockDeleter, MockStep as DeleterStep},
    upload_executor::{MockStep, MockUploadExecutor},
};
use miru_agent::data_uploads::retention::Job as DeleteJob;
use miru_agent::data_uploads::upload::errors::ExecutorErr;
use miru_agent::data_uploads::upload::{
    Job, QueueEntry, QueueSnapshot, QueueSnapshotFile, UploadErr, Uploader, UploaderExt,
    UploaderOptions,
};
use miru_agent::errors::Error;
use miru_agent::filesys::{dirs, File};
use miru_agent::models::FileRuleRetention;

// external crates
use chrono::{DateTime, TimeDelta, Utc};
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
        MockDeleter::new(),
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
    let (uploader, handle, sleeps, _clock) =
        spawn_with_test_clock_and_deleter(mock, MockDeleter::new(), options);
    (uploader, handle, sleeps)
}

/// [`spawn_with_test_clock`] with an injected deleter and the clock exposed,
/// for the confirm-time retention-producer tests: the clock's value when an
/// upload confirms is the `last_observed_at` the enqueued delete job must
/// carry.
#[allow(clippy::type_complexity)]
fn spawn_with_test_clock_and_deleter(
    mock: Arc<MockUploadExecutor>,
    deleter: Arc<MockDeleter>,
    options: UploaderOptions,
) -> (
    Uploader,
    JoinHandle<()>,
    Arc<Mutex<Vec<Duration>>>,
    Arc<Mutex<DateTime<Utc>>>,
) {
    let clock = Arc::new(Mutex::new(Utc::now()));
    let sleeps: Arc<Mutex<Vec<Duration>>> = Arc::new(Mutex::new(Vec::new()));
    let now_clock = clock.clone();
    let now_fn = move || *now_clock.lock().unwrap();
    let recorded = sleeps.clone();
    let sleep_clock = clock.clone();
    let sleep_fn = move |duration: Duration| {
        recorded.lock().unwrap().push(duration);
        *sleep_clock.lock().unwrap() += TimeDelta::from_std(duration).unwrap();
        async {}
    };
    let (uploader, handle) =
        Uploader::spawn(16, mock, deleter, options, None, sleep_fn, now_fn).unwrap();
    (uploader, handle, sleeps, clock)
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
        MockDeleter::new(),
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
async fn network_failure_with_implausible_age_keeps_job() {
    let (mock, mut started_rx) = MockUploadExecutor::new();
    mock.push_step(MockStep::NetworkErr);
    mock.push_step(MockStep::Ok);
    let (uploader, handle, _sleeps) =
        spawn_with_test_clock(mock.clone(), UploaderOptions::default());
    // stamped by a device that booted with an unset clock (1970) before NTP
    // stepped it forward: the computed age is decades, which is a clock
    // artifact rather than an old file, so the backstop must not fire
    let mut job_a = make_job("a.log");
    job_a.first_observed_at = DateTime::from_timestamp(0, 0).unwrap();

    timed(uploader.enqueue(job_a.clone())).await.unwrap();
    for _ in 0..2 {
        timed(started_rx.recv()).await.unwrap();
    }

    timed(uploader.shutdown()).await.unwrap();
    timed(handle).await.unwrap();
    // requeued after the network failure and retried, not dropped
    assert_eq!(mock.recorded_calls(), vec![job_a.clone(), job_a]);
}

#[tokio::test]
async fn implausible_age_boundary_is_pinned_from_both_sides() {
    let (mock, mut started_rx) = MockUploadExecutor::new();
    // A's single failure drops it; B's is exempt, so B fails and then succeeds
    for step in [MockStep::NetworkErr, MockStep::NetworkErr, MockStep::Ok] {
        mock.push_step(step);
    }
    // pinned independently of the production defaults
    let options = UploaderOptions {
        max_job_age: TimeDelta::days(7),
        max_plausible_job_age: TimeDelta::days(30),
        ..UploaderOptions::default()
    };
    let (uploader, handle, _sleeps) = spawn_with_test_clock(mock.clone(), options);
    // past the backstop but still a believable file age: dropped
    let mut job_a = make_job("a.log");
    job_a.first_observed_at = Utc::now() - TimeDelta::days(29);
    // beyond the believable bound: read as clock skew and kept
    let mut job_b = make_job("b.log");
    job_b.first_observed_at = Utc::now() - TimeDelta::days(31);

    timed(uploader.enqueue(job_a.clone())).await.unwrap();
    timed(started_rx.recv()).await.unwrap();
    timed(uploader.enqueue(job_b.clone())).await.unwrap();
    for _ in 0..2 {
        timed(started_rx.recv()).await.unwrap();
    }

    timed(uploader.shutdown()).await.unwrap();
    timed(handle).await.unwrap();
    // A ran once and left; B ran twice, so only B survived its network failure
    assert_eq!(
        mock.recorded_calls(),
        vec![job_a, job_b.clone(), job_b],
        "at the bound the job must drop; beyond it the job must survive"
    );
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
        MockDeleter::new(),
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

/// The confirm-time retention producer: a confirmed upload whose retention
/// requires the upload enqueues a delete job on the deleter, with the TTL
/// counting from the confirm instant.
mod retention_producer {
    use super::*;

    /// A retention block requiring the upload, with `ttl_secs`.
    fn required(ttl_secs: u64) -> Option<FileRuleRetention> {
        Some(FileRuleRetention {
            require_upload: true,
            ttl_secs,
        })
    }

    /// The delete job the producer must build from `job` at confirm time:
    /// every identity field copies off the upload job, `ttl_secs` comes from
    /// its retention, and `last_observed_at` is the confirm instant.
    fn expected_delete_job(job: &Job, confirmed_at: DateTime<Utc>) -> DeleteJob {
        DeleteJob {
            file: job.file.clone(),
            size: job.size,
            digest: job.digest.clone(),
            mtime: job.mtime,
            first_observed_at: job.first_observed_at,
            last_observed_at: confirmed_at,
            ttl_secs: job.retention.as_ref().unwrap().ttl_secs,
            file_rule_id: job.file_rule_id.clone(),
            deployment_id: job.deployment_id.clone(),
        }
    }

    /// Wait until the queue is empty: the job's `queue.remove` — which runs
    /// after the producer's enqueue — has happened.
    async fn await_drained(uploader: &Uploader) {
        timed(async {
            while uploader.len().await.unwrap() > 0 {
                tokio::task::yield_now().await;
            }
        })
        .await;
    }

    #[tokio::test]
    async fn confirmed_upload_with_required_retention_enqueues_a_delete_job() {
        let (mock, mut started_rx) = MockUploadExecutor::new();
        mock.push_step(MockStep::Ok);
        let deleter = MockDeleter::new();
        let (uploader, handle, _sleeps, clock) = spawn_with_test_clock_and_deleter(
            mock.clone(),
            deleter.clone(),
            UploaderOptions::default(),
        );
        let mut job = make_job("a.log");
        job.retention = required(300);

        timed(uploader.enqueue(job.clone())).await.unwrap();
        timed(started_rx.recv()).await.unwrap();
        await_drained(&uploader).await;

        timed(uploader.shutdown()).await.unwrap();
        timed(handle).await.unwrap();
        // no sleeps ran, so the clock still holds the confirm-time value
        let confirmed_at = *clock.lock().unwrap();
        assert_eq!(
            deleter.recorded_calls(),
            [expected_delete_job(&job, confirmed_at)]
        );
    }

    // TTL counts from eligibility (the confirm instant), not from the
    // scan-time observation: a retried upload's delete job is stamped with
    // the clock as of the confirming attempt, after the backoff advanced it.
    #[tokio::test]
    async fn retried_upload_enqueues_once_at_confirm_time() {
        let (mock, mut started_rx) = MockUploadExecutor::new();
        mock.push_step(MockStep::Err);
        mock.push_step(MockStep::Ok);
        let deleter = MockDeleter::new();
        let (uploader, handle, sleeps, clock) = spawn_with_test_clock_and_deleter(
            mock.clone(),
            deleter.clone(),
            UploaderOptions::default(),
        );
        let mut job = make_job("a.log");
        job.retention = required(300);

        timed(uploader.enqueue(job.clone())).await.unwrap();
        for _ in 0..2 {
            timed(started_rx.recv()).await.unwrap();
        }
        await_drained(&uploader).await;

        timed(uploader.shutdown()).await.unwrap();
        timed(handle).await.unwrap();
        // the failed attempt enqueued nothing; the confirm-time stamp sits
        // after the backoff sleep the retry waited out
        assert!(!sleeps.lock().unwrap().is_empty());
        let confirmed_at = *clock.lock().unwrap();
        assert_eq!(
            deleter.recorded_calls(),
            [expected_delete_job(&job, confirmed_at)]
        );
    }

    // require_upload: false files were already enqueued at stability by the
    // retention sink; the confirm-time enqueue is a same-path duplicate, which
    // the queue permits and the sweep resolves harmlessly. Any retention on a
    // confirmed upload produces a delete job.
    #[tokio::test]
    async fn unrequired_retention_also_enqueues_a_delete_job() {
        let (mock, mut started_rx) = MockUploadExecutor::new();
        mock.push_step(MockStep::Ok);
        let deleter = MockDeleter::new();
        let (uploader, handle, _sleeps, clock) = spawn_with_test_clock_and_deleter(
            mock.clone(),
            deleter.clone(),
            UploaderOptions::default(),
        );
        let mut job = make_job("a.log");
        job.retention = Some(FileRuleRetention {
            require_upload: false,
            ttl_secs: 300,
        });

        timed(uploader.enqueue(job.clone())).await.unwrap();
        timed(started_rx.recv()).await.unwrap();
        await_drained(&uploader).await;

        timed(uploader.shutdown()).await.unwrap();
        timed(handle).await.unwrap();
        let confirmed_at = *clock.lock().unwrap();
        assert_eq!(
            deleter.recorded_calls(),
            [expected_delete_job(&job, confirmed_at)]
        );
    }

    #[tokio::test]
    async fn absent_retention_enqueues_nothing() {
        let (mock, mut started_rx) = MockUploadExecutor::new();
        mock.push_step(MockStep::Ok);
        let deleter = MockDeleter::new();
        let (uploader, handle, _sleeps, _clock) = spawn_with_test_clock_and_deleter(
            mock.clone(),
            deleter.clone(),
            UploaderOptions::default(),
        );

        // make_job carries no retention
        timed(uploader.enqueue(make_job("a.log"))).await.unwrap();
        timed(started_rx.recv()).await.unwrap();
        await_drained(&uploader).await;

        timed(uploader.shutdown()).await.unwrap();
        timed(handle).await.unwrap();
        assert_eq!(deleter.recorded_calls(), []);
    }

    // a terminal failure never confirms, so it must never produce a delete
    // job no matter what the retention says.
    #[tokio::test]
    async fn terminal_failure_enqueues_nothing() {
        let (mock, mut started_rx) = MockUploadExecutor::new();
        mock.push_step(MockStep::TerminalErr);
        let deleter = MockDeleter::new();
        let (uploader, handle, _sleeps, _clock) = spawn_with_test_clock_and_deleter(
            mock.clone(),
            deleter.clone(),
            UploaderOptions::default(),
        );
        let mut job = make_job("a.log");
        job.retention = required(0);

        timed(uploader.enqueue(job)).await.unwrap();
        timed(started_rx.recv()).await.unwrap();
        await_drained(&uploader).await;

        timed(uploader.shutdown()).await.unwrap();
        timed(handle).await.unwrap();
        assert_eq!(deleter.recorded_calls(), []);
    }

    // enqueue failures are swallowed: the upload is already durably
    // confirmed, so a dead deleter must not fail the attempt or the worker.
    #[tokio::test]
    async fn enqueue_failure_is_swallowed_and_worker_continues() {
        let (mock, mut started_rx) = MockUploadExecutor::new();
        mock.push_step(MockStep::Ok);
        mock.push_step(MockStep::Ok);
        let deleter = MockDeleter::new();
        deleter.push_step(DeleterStep::Err);
        let (uploader, handle, _sleeps, clock) = spawn_with_test_clock_and_deleter(
            mock.clone(),
            deleter.clone(),
            UploaderOptions::default(),
        );
        let mut job_a = make_job("a.log");
        job_a.retention = required(60);
        let mut job_b = make_job("b.log");
        job_b.retention = required(60);

        timed(uploader.enqueue(job_a.clone())).await.unwrap();
        timed(started_rx.recv()).await.unwrap();
        // a second job still runs and produces its delete job: the failed
        // enqueue neither re-drove A's upload nor wedged the worker
        timed(uploader.enqueue(job_b.clone())).await.unwrap();
        timed(started_rx.recv()).await.unwrap();
        await_drained(&uploader).await;

        timed(uploader.shutdown()).await.unwrap();
        timed(handle).await.unwrap();
        assert_eq!(mock.recorded_calls(), vec![job_a.clone(), job_b.clone()]);
        let confirmed_at = *clock.lock().unwrap();
        assert_eq!(
            deleter.recorded_calls(),
            [
                expected_delete_job(&job_a, confirmed_at),
                expected_delete_job(&job_b, confirmed_at),
            ]
        );
    }
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
            MockDeleter::new(),
            UploaderOptions::default(),
            Some(snapshot),
            |_: Duration| async {},
            Utc::now,
        )
        .unwrap()
    }

    /// [`spawn_persisted`] over the shared test clock, so an idle wait completes
    /// instantly and the duration it asked for is recorded.
    fn spawn_persisted_with_test_clock(
        mock: Arc<MockUploadExecutor>,
        snapshot: QueueSnapshotFile,
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
        let (uploader, handle) = Uploader::spawn(
            16,
            mock,
            MockDeleter::new(),
            UploaderOptions::default(),
            Some(snapshot),
            sleep_fn,
            now_fn,
        )
        .unwrap();
        (uploader, handle, sleeps)
    }

    /// Seed `path` with one entry stamped `deadline`, as a snapshot written
    /// before a clock step would look.
    async fn seed(path: &File, deadline: DateTime<Utc>) -> Job {
        let job = make_job("stranded.log");
        let mut snapshot = open(path).await;
        snapshot
            .patch(QueueSnapshot {
                entries: vec![QueueEntry {
                    id: uuid::Uuid::new_v4(),
                    job: job.clone(),
                    attempts: 1,
                    next_attempt_at: Some(deadline),
                }],
            })
            .await
            .unwrap();
        job
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

    /// The offline backlog is exactly what the durable queue exists to protect,
    /// so a clock step must not erase it from disk.
    #[tokio::test]
    async fn implausible_age_keeps_job_on_disk() {
        let dir = dirs::temp("uploader_clock_skew").unwrap();
        let path = dir.to_dir().file("upload_queue.json");
        let (mock, mut started_rx) = MockUploadExecutor::new();
        let (_release_tx, release_rx) = oneshot::channel();
        // fail the network on the first attempt, then hang the retry so the
        // snapshot can be read while the job is still queued
        mock.push_step(MockStep::NetworkErr);
        mock.push_step(MockStep::Hang(release_rx));
        let (uploader, handle, _sleeps) =
            spawn_persisted_with_test_clock(mock.clone(), open(&path).await);

        let mut job = make_job("a.log");
        job.first_observed_at = DateTime::from_timestamp(0, 0).unwrap();
        timed(uploader.enqueue(job)).await.unwrap();
        for _ in 0..2 {
            timed(started_rx.recv()).await.unwrap();
        }

        assert_eq!(on_disk(&path).await, vec!["sha256:a.log".to_string()]);

        timed(uploader.shutdown()).await.unwrap();
        timed(handle).await.unwrap();
    }

    #[tokio::test]
    async fn a_deadline_past_the_max_backoff_is_pulled_back_before_sleeping() {
        let dir = dirs::temp("uploader_stranded").unwrap();
        let path = dir.to_dir().file("upload_queue.json");
        // 48h out: unreachable under the default 1h maximum backoff, so only a
        // clock step could have produced it
        seed(&path, Utc::now() + TimeDelta::hours(48)).await;
        let (mock, mut started_rx) = MockUploadExecutor::new();
        mock.push_step(MockStep::Ok);

        let (uploader, handle, sleeps) =
            spawn_persisted_with_test_clock(mock.clone(), open(&path).await);

        // the worker waits one maximum backoff, not 48 hours, and then uploads
        timed(started_rx.recv()).await.unwrap();
        assert_eq!(
            *sleeps.lock().unwrap(),
            vec![Duration::from_secs(3600)],
            "the stranded deadline should have been pulled back to the horizon"
        );

        timed(uploader.shutdown()).await.unwrap();
        timed(handle).await.unwrap();
    }

    #[tokio::test]
    async fn a_deadline_inside_the_max_backoff_is_left_alone() {
        let dir = dirs::temp("uploader_backoff_kept").unwrap();
        let path = dir.to_dir().file("upload_queue.json");
        // an ordinary backoff stamp: the worker must honor it rather than
        // treating every future deadline as a clock anomaly
        seed(&path, Utc::now() + TimeDelta::seconds(60)).await;
        let (mock, mut started_rx) = MockUploadExecutor::new();
        mock.push_step(MockStep::Ok);

        let (uploader, handle, sleeps) =
            spawn_persisted_with_test_clock(mock.clone(), open(&path).await);

        timed(started_rx.recv()).await.unwrap();
        let waited = sleeps.lock().unwrap().first().copied().unwrap();
        assert!(
            waited <= Duration::from_secs(60),
            "an in-range backoff must be honored, waited {waited:?}"
        );

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
