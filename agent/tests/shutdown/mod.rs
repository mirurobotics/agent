// standard crates
use std::future::Future;
use std::pin::{pin, Pin};
use std::task::{Context, Poll, Waker};

// internal crates
use miru_agent::shutdown::Latch;

fn poll_once<F: Future>(fut: Pin<&mut F>) -> Poll<F::Output> {
    fut.poll(&mut Context::from_waker(Waker::noop()))
}

fn assert_pending<F: Future>(fut: Pin<&mut F>) {
    assert!(poll_once(fut).is_pending());
}

fn assert_ready(fut: Pin<&mut impl Future<Output = ()>>) {
    assert_eq!(Poll::Ready(()), poll_once(fut));
}

#[test]
fn trigger_resolves_two_independent_waiters() {
    let latch = Latch::new();
    let mut first = pin!(latch.wait());
    let mut second = pin!(latch.wait());

    assert_pending(first.as_mut());
    assert_pending(second.as_mut());

    latch.trigger();

    assert_ready(first.as_mut());
    assert_ready(second.as_mut());
}

#[test]
fn wait_created_after_trigger_resolves_at_once() {
    let latch = Latch::new();
    latch.trigger();

    assert_ready(pin!(latch.wait()).as_mut());
}

#[test]
fn untriggered_wait_stays_pending() {
    let latch = Latch::new();
    let mut waiter = pin!(latch.wait());

    assert_pending(waiter.as_mut());
    assert!(!latch.is_triggered());
}

#[test]
fn repeated_trigger_is_harmless() {
    let latch = Latch::new();
    let mut waiter = pin!(latch.wait());

    latch.trigger();
    latch.trigger();

    assert_ready(waiter.as_mut());
    assert!(latch.is_triggered());
}

#[test]
fn trigger_from_plain_thread_wakes_async_waiter() {
    let latch = Latch::new();
    let mut waiter = pin!(latch.wait());
    assert_pending(waiter.as_mut());

    let from_thread = latch.clone();
    std::thread::spawn(move || from_thread.trigger())
        .join()
        .expect("trigger thread joins");

    assert_ready(waiter.as_mut());
}

#[test]
fn is_triggered_flips_from_false_to_true() {
    let latch = Latch::new();
    assert!(!latch.is_triggered());

    latch.trigger();

    assert!(latch.is_triggered());
}

#[test]
fn default_behaves_like_new() {
    let latch = Latch::default();
    assert!(!latch.is_triggered());

    latch.trigger();

    assert!(latch.is_triggered());
    assert_ready(pin!(latch.wait()).as_mut());
}

#[test]
fn clones_share_one_latch() {
    let latch = Latch::new();
    let clone = latch.clone();
    let mut waiter = pin!(clone.wait());

    latch.trigger();

    assert!(clone.is_triggered());
    assert_ready(waiter.as_mut());
}

#[test]
fn dropping_every_handle_resolves_waiters() {
    let latch = Latch::new();
    let mut waiter = pin!(latch.wait());
    assert_pending(waiter.as_mut());

    drop(latch);

    assert_ready(waiter.as_mut());
}
