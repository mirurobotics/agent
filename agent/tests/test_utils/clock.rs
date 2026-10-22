// standard crates
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// internal crates
use miru_agent::clock::Clock;

// external crates
use chrono::{DateTime, TimeDelta, Utc};

/// A [`Clock`] whose system and monotonic sources a test moves by hand.
/// Clones share the same time.
#[derive(Clone)]
pub struct FakeClock {
    system: Arc<Mutex<DateTime<Utc>>>,
    monotonic: Arc<Mutex<Instant>>,
    clock: Clock,
}

impl FakeClock {
    /// A clock whose system clock reads `system` and whose monotonic clock
    /// starts at the real current instant.
    pub fn new(system: DateTime<Utc>) -> Self {
        let system = Arc::new(Mutex::new(system));
        let monotonic = Arc::new(Mutex::new(Instant::now()));
        let (s, m) = (system.clone(), monotonic.clone());
        let clock = Clock::from_sources(
            Arc::new(move || *s.lock().unwrap()),
            Arc::new(move || *m.lock().unwrap()),
        );
        Self {
            system,
            monotonic,
            clock,
        }
    }

    /// A clock whose system clock reads `secs` seconds after the Unix epoch.
    pub fn at_secs(secs: i64) -> Self {
        Self::new(DateTime::from_timestamp(secs, 0).unwrap())
    }

    /// The [`Clock`] to hand to the code under test.
    pub fn clock(&self) -> Clock {
        self.clock.clone()
    }

    /// Advance both clocks together, as real time passing does.
    pub fn advance(&self, d: Duration) {
        *self.monotonic.lock().unwrap() += d;
        *self.system.lock().unwrap() += TimeDelta::from_std(d).unwrap();
    }

    /// Advance both clocks by `secs` seconds.
    pub fn advance_secs(&self, secs: u64) {
        self.advance(Duration::from_secs(secs));
    }

    /// Jump only the system clock, as NTP correcting it does. The monotonic
    /// clock does not move.
    pub fn set_system(&self, to: DateTime<Utc>) {
        *self.system.lock().unwrap() = to;
    }

    /// The system clock's current reading.
    pub fn system_now(&self) -> DateTime<Utc> {
        *self.system.lock().unwrap()
    }

    /// The monotonic clock's current reading.
    pub fn monotonic_now(&self) -> Instant {
        *self.monotonic.lock().unwrap()
    }
}
