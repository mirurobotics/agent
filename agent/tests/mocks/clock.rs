// standard crates
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// internal crates
use miru_agent::clock::Clock;

// external crates
use chrono::{DateTime, TimeDelta, Utc};

/// A clock whose `sleep` advances its own `now` by the slept duration and
/// records it. Waits complete instantly and backoff-stamped entries become
/// eligible deterministically, because the same time source backs both
/// halves — the advance is not something a caller can forget to wire up.
#[derive(Clone)]
pub struct TestClock {
    now: Arc<Mutex<DateTime<Utc>>>,
    sleeps: Arc<Mutex<Vec<Duration>>>,
}

impl TestClock {
    pub fn new() -> Self {
        Self {
            now: Arc::new(Mutex::new(Utc::now())),
            sleeps: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Every duration passed to `sleep`, in order.
    pub fn sleeps(&self) -> Arc<Mutex<Vec<Duration>>> {
        self.sleeps.clone()
    }

    /// The live `now` cell, for tests asserting on the time an upload
    /// confirmed at.
    pub fn handle(&self) -> Arc<Mutex<DateTime<Utc>>> {
        self.now.clone()
    }
}

impl Default for TestClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for TestClock {
    fn now(&self) -> DateTime<Utc> {
        *self.now.lock().unwrap()
    }

    fn sleep(&self, dur: Duration) -> impl Future<Output = ()> + Send {
        self.sleeps.lock().unwrap().push(dur);
        *self.now.lock().unwrap() += TimeDelta::from_std(dur).unwrap();
        std::future::ready(())
    }
}

/// A clock stopped at construction: `now` never moves and `sleep` never
/// resolves. Backoff-stamped entries stay ineligible, so the worker parks in
/// its idle wait until a command arrives.
#[derive(Clone, Copy)]
pub struct FrozenClock {
    at: DateTime<Utc>,
}

impl FrozenClock {
    pub fn new() -> Self {
        Self { at: Utc::now() }
    }
}

impl Default for FrozenClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for FrozenClock {
    fn now(&self) -> DateTime<Utc> {
        self.at
    }

    fn sleep(&self, _dur: Duration) -> impl Future<Output = ()> + Send {
        std::future::pending()
    }
}
