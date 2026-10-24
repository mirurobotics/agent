// standard crates
use std::sync::Arc;
use std::time::Instant;

// internal crates
use miru_agent::clock::Clock;

// external crates
use chrono::{DateTime, Utc};

/// A system clock stuck at 1970, as on a robot that booted without a
/// battery-backed clock.
pub fn system_time() -> DateTime<Utc> {
    DateTime::<Utc>::UNIX_EPOCH
}

/// The time the backend reports in [`server_clock`].
pub fn server_time() -> DateTime<Utc> {
    "2026-10-21T12:00:00Z".parse().unwrap()
}

/// A clock that has heard [`server_time`] from the backend while its system
/// clock reads [`system_time`]. Monotonic time stands still, so the server
/// time it reports never moves.
pub fn server_clock() -> Clock {
    let monotonic = Instant::now();
    let clock = Clock::from_sources(Arc::new(system_time), Arc::new(move || monotonic));
    clock.observe_server_time(server_time());
    clock
}
