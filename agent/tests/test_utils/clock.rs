// standard crates
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// internal crates
use miru_agent::clock::{Clock, Uptime};

// external crates
use chrono::{DateTime, TimeDelta, Utc};

/// A [`Clock`] whose system and monotonic clocks a test moves by hand.
/// Clones share the same clocks.
#[derive(Clone)]
pub struct FakeClock {
    system: Arc<Mutex<DateTime<Utc>>>,
    monotonic: Arc<Mutex<Instant>>,
    clock: Clock,
}

impl FakeClock {
    /// A clock whose system clock reads `system_secs` past the epoch and
    /// which has not heard from the server yet.
    pub fn new(system_secs: i64) -> Self {
        let system = Arc::new(Mutex::new(at(system_secs)));
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

    /// The [`Clock`] to hand to the code under test.
    pub fn clock(&self) -> Clock {
        self.clock.clone()
    }

    /// An [`Uptime`] on this clock, counting from zero.
    pub fn uptime(&self) -> Uptime {
        Uptime::new(self.clock(), Duration::ZERO)
    }

    /// Advance the system and monotonic clocks together by `secs`, as real
    /// time passing does.
    pub fn advance(&self, secs: u64) {
        *self.monotonic.lock().unwrap() += Duration::from_secs(secs);
        *self.system.lock().unwrap() += TimeDelta::seconds(secs as i64);
    }

    /// Set the system clock to `secs` past the epoch without moving the
    /// monotonic clock, as NTP correcting it does.
    pub fn set_system(&self, secs: i64) {
        *self.system.lock().unwrap() = at(secs);
    }

    /// Record that the server's time is `secs` past the epoch right now.
    pub fn observe_server(&self, secs: i64) {
        self.clock.observe_server_time(at(secs));
    }

    /// A fresh process on the same robot: the system clock carries on from
    /// here, but monotonic time restarts and the server has not been heard
    /// from yet.
    pub fn restart(&self) -> Self {
        let restarted = Self::new(0);
        *restarted.system.lock().unwrap() = *self.system.lock().unwrap();
        restarted
    }
}

/// `secs` past the epoch.
fn at(secs: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(secs, 0).unwrap()
}
