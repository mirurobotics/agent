// standard crates
use std::time::Duration;

// internal crates
use miru_agent::shutdown::Latch;

// external crates
use tokio::time::timeout;

const RESOLVE_WITHIN: Duration = Duration::from_secs(1);
const STAYS_PENDING_FOR: Duration = Duration::from_millis(100);

#[tokio::test]
async fn trigger_resolves_two_independent_waiters() {
    let latch = Latch::new();
    let first = latch.wait();
    let second = latch.wait();

    latch.trigger();

    timeout(RESOLVE_WITHIN, first)
        .await
        .expect("first waiter resolves");
    timeout(RESOLVE_WITHIN, second)
        .await
        .expect("second waiter resolves");
}

#[tokio::test]
async fn wait_created_after_trigger_resolves_at_once() {
    let latch = Latch::new();
    latch.trigger();

    timeout(RESOLVE_WITHIN, latch.wait())
        .await
        .expect("late waiter resolves immediately");
}

#[tokio::test]
async fn untriggered_wait_stays_pending() {
    let latch = Latch::new();

    let result = timeout(STAYS_PENDING_FOR, latch.wait()).await;

    assert!(result.is_err(), "wait() must not resolve before trigger()");
    assert!(!latch.is_triggered());
}

#[tokio::test]
async fn repeated_trigger_is_harmless() {
    let latch = Latch::new();
    let waiter = latch.wait();

    latch.trigger();
    latch.trigger();

    timeout(RESOLVE_WITHIN, waiter)
        .await
        .expect("waiter resolves after repeated trigger");
    assert!(latch.is_triggered());
}

#[tokio::test]
async fn trigger_from_plain_thread_wakes_async_waiter() {
    let latch = Latch::new();
    let waiter = latch.wait();

    let from_thread = latch.clone();
    let thread = std::thread::spawn(move || from_thread.trigger());

    timeout(RESOLVE_WITHIN, waiter)
        .await
        .expect("waiter woken by a non-tokio thread");
    thread.join().expect("trigger thread joins");
}

#[tokio::test]
async fn is_triggered_flips_from_false_to_true() {
    let latch = Latch::new();
    assert!(!latch.is_triggered());

    latch.trigger();

    assert!(latch.is_triggered());
}

#[tokio::test]
async fn default_behaves_like_new() {
    let latch = Latch::default();
    assert!(!latch.is_triggered());

    latch.trigger();

    assert!(latch.is_triggered());
    timeout(RESOLVE_WITHIN, latch.wait())
        .await
        .expect("default-constructed signal resolves waiters");
}

#[tokio::test]
async fn clones_share_one_signal() {
    let latch = Latch::new();
    let clone = latch.clone();
    let waiter = clone.wait();

    latch.trigger();

    assert!(clone.is_triggered());
    timeout(RESOLVE_WITHIN, waiter)
        .await
        .expect("waiter on the clone resolves");
}

#[tokio::test]
async fn dropping_every_handle_resolves_waiters() {
    let latch = Latch::new();
    let waiter = latch.wait();

    drop(latch);

    timeout(RESOLVE_WITHIN, waiter)
        .await
        .expect("waiter resolves once no handle can trigger the signal");
}
