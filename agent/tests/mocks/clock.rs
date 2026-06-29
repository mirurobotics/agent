// standard crates
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

// external crates
use chrono::{DateTime, Utc};

/// A controllable monotonic-ish clock for deterministic worker tests. Holds the
/// current time as epoch seconds in a shared atomic so a test can step it
/// forward independently of wall-clock time. `now_fn()` produces the
/// `Fn() -> DateTime<Utc>` closure that the worker's injected `now_fn` expects.
#[derive(Clone)]
pub struct Clock {
    secs: Arc<AtomicI64>,
}

impl Clock {
    pub fn new(start_secs: i64) -> Self {
        Self {
            secs: Arc::new(AtomicI64::new(start_secs)),
        }
    }

    pub fn now_fn(&self) -> impl Fn() -> DateTime<Utc> {
        let secs = self.secs.clone();
        move || DateTime::from_timestamp(secs.load(Ordering::SeqCst), 0).unwrap()
    }

    pub fn advance(&self, secs: i64) {
        self.secs.fetch_add(secs, Ordering::SeqCst);
    }
}
