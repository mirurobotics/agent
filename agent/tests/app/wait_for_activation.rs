// standard crates
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration as StdDuration;

// internal crates
use miru_agent::app::wait_for_activation::{should_log, wait_for_activation, WaitOutcome};
use miru_agent::filesys::{self, WriteOptions};
use miru_agent::storage::Layout;

// external crates
// (none — stdlib + tokio macros)

// ============================ TEST HARNESS ============================ //

async fn fresh_layout(name: &str) -> (Layout, filesys::Dir) {
    let dir = filesys::Dir::create_temp_dir(name).await.unwrap();
    let layout = Layout::new(dir.clone());
    layout.auth().root.create_if_absent().await.unwrap();
    (layout, dir)
}

async fn write_keys(layout: &Layout) {
    let auth = layout.auth();
    auth.private_key()
        .write_string("private", WriteOptions::OVERWRITE_ATOMIC)
        .await
        .unwrap();
    auth.public_key()
        .write_string("public", WriteOptions::OVERWRITE_ATOMIC)
        .await
        .unwrap();
}

// ============================ TESTS ============================ //

#[tokio::test]
async fn activates_immediately_when_keys_already_present() {
    let (layout, _dir) = fresh_layout("wait_activates_immediately").await;
    write_keys(&layout).await;

    let sleep_count = Arc::new(AtomicUsize::new(0));
    let counter = sleep_count.clone();
    let sleep_fn = move |_: StdDuration| {
        let counter = counter.clone();
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
        }
    };

    let shutdown = std::future::pending::<()>();

    let outcome = wait_for_activation(&layout, sleep_fn, shutdown).await;

    assert_eq!(outcome, WaitOutcome::Activated);
    assert_eq!(
        sleep_count.load(Ordering::SeqCst),
        0,
        "should not sleep even once when activation is already complete",
    );
}

#[tokio::test]
async fn activates_after_n_cycles() {
    let (layout, dir) = fresh_layout("wait_activates_after_n").await;

    let activate_after: usize = 3;
    let sleep_count = Arc::new(AtomicUsize::new(0));
    let counter = sleep_count.clone();
    let layout_for_sleep = Layout::new(dir.clone());

    // The sleep_fn writes the keys on its Nth invocation. Ordering inside the
    // production loop is: assert_activated → sleep → assert_activated → sleep
    // → ... so writing keys during the Nth sleep means the (N+1)th
    // assert_activated check sees them.
    let sleep_fn = move |_: StdDuration| {
        let n = counter.fetch_add(1, Ordering::SeqCst);
        let layout = layout_for_sleep.clone();
        async move {
            if n + 1 == activate_after {
                let auth = layout.auth();
                auth.private_key()
                    .write_string("private", WriteOptions::OVERWRITE_ATOMIC)
                    .await
                    .unwrap();
                auth.public_key()
                    .write_string("public", WriteOptions::OVERWRITE_ATOMIC)
                    .await
                    .unwrap();
            }
        }
    };

    let shutdown = std::future::pending::<()>();
    let outcome = wait_for_activation(&layout, sleep_fn, shutdown).await;

    assert_eq!(outcome, WaitOutcome::Activated);
    assert_eq!(
        sleep_count.load(Ordering::SeqCst),
        activate_after,
        "should sleep exactly N times: assert_activated misses N times, sleep injects keys on Nth, next check succeeds",
    );
}

#[tokio::test]
async fn shutdown_during_wait_returns_shutdown_requested() {
    let (layout, _dir) = fresh_layout("wait_shutdown_during").await;
    // No keys are ever created.

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    let sleep_count = Arc::new(AtomicUsize::new(0));
    let counter = sleep_count.clone();
    let shutdown_tx = Arc::new(tokio::sync::Mutex::new(Some(shutdown_tx)));
    let shutdown_tx_for_sleep = shutdown_tx.clone();

    // After 5 sleeps, fire shutdown.
    let sleep_fn = move |_: StdDuration| {
        let n = counter.fetch_add(1, Ordering::SeqCst);
        let tx = shutdown_tx_for_sleep.clone();
        async move {
            if n + 1 == 5 {
                if let Some(tx) = tx.lock().await.take() {
                    let _ = tx.send(());
                }
            }
        }
    };

    let shutdown = async move {
        let _ = shutdown_rx.await;
    };

    let outcome = wait_for_activation(&layout, sleep_fn, shutdown).await;

    assert_eq!(outcome, WaitOutcome::ShutdownRequested);
    // Sleep count is "≥ 5" not "== 5" because tokio::select! polls both
    // arms; a final sleep_fn invocation may have started before shutdown won.
    assert!(
        sleep_count.load(Ordering::SeqCst) >= 5,
        "expected at least 5 sleeps before shutdown fired",
    );
}

#[tokio::test]
async fn shutdown_wins_when_already_signaled_at_entry() {
    // No keys, shutdown future is already-resolved when we enter the loop.
    let (layout, _dir) = fresh_layout("wait_shutdown_immediate").await;

    let sleep_count = Arc::new(AtomicUsize::new(0));
    let counter = sleep_count.clone();
    let sleep_fn = move |_: StdDuration| {
        let counter = counter.clone();
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
        }
    };

    let shutdown = std::future::ready(());

    let outcome = wait_for_activation(&layout, sleep_fn, shutdown).await;

    assert_eq!(outcome, WaitOutcome::ShutdownRequested);
    // `biased; shutdown` first means we should not have slept at all:
    // the first `tokio::select!` iteration sees `shutdown` ready and wins.
    assert_eq!(sleep_count.load(Ordering::SeqCst), 0);
}

#[test]
fn should_log_is_publicly_reachable_and_matches_unit_schedule() {
    // The full schedule is covered in unit tests next to the source.
    // This test just locks in the public-API path.
    assert!(!should_log(0));
    assert!(should_log(2));
    assert!(should_log(1024));
    assert!(should_log(2048));
    assert!(!should_log(2049));
}
