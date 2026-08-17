// standard crates
use std::future::Future;
use std::time::Duration;

// external crates
use chrono::{DateTime, Utc};

/// A single source of time: reading it and waiting on it.
///
/// The two are one trait because they must agree. A `sleep` that resolves
/// without `now` advancing leaves a backoff-stamped entry permanently
/// ineligible, and a run loop that waits for the earliest deadline spins on
/// it — starving the runtime rather than failing a test. Implementing both
/// halves against one time source is what makes that pairing hard to build
/// by accident.
pub trait Clock: Clone + Send + Sync + 'static {
    fn now(&self) -> DateTime<Utc>;
    /// The returned future must not borrow `self`; clone the clock first if
    /// the caller needs `self` while the sleep is pending.
    fn sleep(&self, dur: Duration) -> impl Future<Output = ()> + Send;
}

/// Wall clock and real timer.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }

    fn sleep(&self, dur: Duration) -> impl Future<Output = ()> + Send {
        tokio::time::sleep(dur)
    }
}
