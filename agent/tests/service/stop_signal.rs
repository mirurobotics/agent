// standard crates
use std::time::Duration;

// internal crates
use miru_agent::service::StopSignal;

// external crates
use tokio::time::timeout;

const RESOLVE_WITHIN: Duration = Duration::from_secs(1);
const STAYS_PENDING_FOR: Duration = Duration::from_millis(100);

#[tokio::test]
async fn trigger_resolves_two_independent_waiters() {
    let stop = StopSignal::new();
    let first = stop.wait();
    let second = stop.wait();

    stop.trigger();

    timeout(RESOLVE_WITHIN, first)
        .await
        .expect("first waiter resolves");
    timeout(RESOLVE_WITHIN, second)
        .await
        .expect("second waiter resolves");
}

#[tokio::test]
async fn wait_created_after_trigger_resolves_at_once() {
    let stop = StopSignal::new();
    stop.trigger();

    timeout(RESOLVE_WITHIN, stop.wait())
        .await
        .expect("late waiter resolves immediately");
}

#[tokio::test]
async fn untriggered_wait_stays_pending() {
    let stop = StopSignal::new();

    let result = timeout(STAYS_PENDING_FOR, stop.wait()).await;

    assert!(result.is_err(), "wait() must not resolve before trigger()");
    assert!(!stop.is_triggered());
}

#[tokio::test]
async fn repeated_trigger_is_harmless() {
    let stop = StopSignal::new();
    let waiter = stop.wait();

    stop.trigger();
    stop.trigger();

    timeout(RESOLVE_WITHIN, waiter)
        .await
        .expect("waiter resolves after repeated trigger");
    assert!(stop.is_triggered());
}

#[tokio::test]
async fn trigger_from_plain_thread_wakes_async_waiter() {
    let stop = StopSignal::new();
    let waiter = stop.wait();

    let from_thread = stop.clone();
    let thread = std::thread::spawn(move || from_thread.trigger());

    timeout(RESOLVE_WITHIN, waiter)
        .await
        .expect("waiter woken by a non-tokio thread");
    thread.join().expect("trigger thread joins");
}

#[tokio::test]
async fn is_triggered_flips_from_false_to_true() {
    let stop = StopSignal::new();
    assert!(!stop.is_triggered());

    stop.trigger();

    assert!(stop.is_triggered());
}

#[tokio::test]
async fn default_behaves_like_new() {
    let stop = StopSignal::default();
    assert!(!stop.is_triggered());

    stop.trigger();

    assert!(stop.is_triggered());
    timeout(RESOLVE_WITHIN, stop.wait())
        .await
        .expect("default-constructed signal resolves waiters");
}

#[tokio::test]
async fn clones_share_one_signal() {
    let stop = StopSignal::new();
    let clone = stop.clone();
    let waiter = clone.wait();

    stop.trigger();

    assert!(clone.is_triggered());
    timeout(RESOLVE_WITHIN, waiter)
        .await
        .expect("waiter on the clone resolves");
}

#[tokio::test]
async fn dropping_every_handle_resolves_waiters() {
    let stop = StopSignal::new();
    let waiter = stop.wait();

    drop(stop);

    timeout(RESOLVE_WITHIN, waiter)
        .await
        .expect("waiter resolves once no handle can trigger the signal");
}
