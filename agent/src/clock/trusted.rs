//! [`Clock`]: elapsed, trusted and raw time from one shared backend reading.

// standard crates
use std::fmt;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

// internal crates
use crate::clock::http_date::{parse_age, parse_http_date};

// external crates
use chrono::{DateTime, TimeDelta, Utc};
use tracing::{debug, warn};

/// How far the wall clock may differ from the backend's time and still count
/// as trusted. The `Date` header has one-second resolution and the response
/// spends some time in flight, so an exact match is not expected.
pub const TRUST_TOLERANCE: TimeDelta = TimeDelta::seconds(60);

type DateTimeFn = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;
type InstantFn = Arc<dyn Fn() -> Instant + Send + Sync>;

/// One reading of the backend's time, with what the agent's own clocks said
/// at the same moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Anchor {
    /// The backend's time, from its `Date` header.
    backend_time: DateTime<Utc>,
    /// The monotonic instant the reading was taken.
    observed_at: Instant,
    /// The robot's wall clock when the reading was taken.
    wall_clock: DateTime<Utc>,
}

/// The agent's shared clock. Cheap to clone; clones share the backend anchor.
#[derive(Clone)]
pub struct Clock {
    datetime_fn: DateTimeFn,
    instant_fn: InstantFn,
    anchor: Arc<RwLock<Option<Anchor>>>,
}

impl fmt::Debug for Clock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Clock")
            .field("anchor", &self.get_anchor())
            .finish_non_exhaustive()
    }
}

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock {
    /// A clock backed by the system wall clock and monotonic clock.
    pub fn new() -> Self {
        Self::from_sources(Arc::new(Utc::now), Arc::new(Instant::now))
    }

    /// A clock with injected wall-clock and monotonic sources, so tests can
    /// control both.
    pub fn from_sources(datetime_fn: DateTimeFn, instant_fn: InstantFn) -> Self {
        Self {
            datetime_fn,
            instant_fn,
            anchor: Arc::new(RwLock::new(None)),
        }
    }

    /// The wall clock as-is. It may be decades off; use it only for logs.
    pub fn raw_now(&self) -> DateTime<Utc> {
        (self.datetime_fn)()
    }

    /// The current monotonic instant.
    pub fn instant(&self) -> Instant {
        (self.instant_fn)()
    }

    /// Monotonic time since `earlier`, or zero if `earlier` is in the future.
    pub fn elapsed_since(&self, earlier: Instant) -> Duration {
        self.instant().saturating_duration_since(earlier)
    }

    /// Records a reading of the backend's time taken now. Later readings
    /// replace earlier ones.
    pub fn observe_server_time(&self, server: DateTime<Utc>) {
        let anchor = Anchor {
            backend_time: server,
            observed_at: self.instant(),
            wall_clock: self.raw_now(),
        };
        let previous = self.set_anchor(anchor);
        // computed up front: arguments of a disabled log macro never run
        let offset_secs = (server - anchor.wall_clock).num_seconds();
        if offset_secs.abs() > TRUST_TOLERANCE.num_seconds() && previous.is_none() {
            warn!(
                "clock: wall clock differs from the backend by {offset_secs}s; using backend time"
            );
        } else {
            debug!("clock: backend time {server}, offset {offset_secs}s");
        }
    }

    /// Records the backend's time from a response's HTTP `Date` header. If a
    /// cache served the response, its `Age` header says how long ago the
    /// backend stamped that `Date`, so the age is added back. A `Date` that does
    /// not parse is ignored; an `Age` that does not parse counts as zero.
    pub fn observe_date_header(&self, date: &str, age: Option<&str>) {
        let Some(server) = parse_http_date(date) else {
            debug!("clock: ignoring unparseable Date header {date:?}");
            return;
        };
        let age = age.and_then(parse_age).unwrap_or_default();
        let server = server
            .checked_add_signed(age)
            .unwrap_or(DateTime::<Utc>::MAX_UTC);
        self.observe_server_time(server);
    }

    /// The backend's time now, or `None` if the agent has not heard from the
    /// backend in this process.
    pub fn trusted_now(&self) -> Option<DateTime<Utc>> {
        let anchor = self.get_anchor()?;
        let elapsed = self.instant().saturating_duration_since(anchor.observed_at);
        let elapsed = TimeDelta::from_std(elapsed).unwrap_or(TimeDelta::MAX);
        Some(
            anchor
                .backend_time
                .checked_add_signed(elapsed)
                .unwrap_or(DateTime::<Utc>::MAX_UTC),
        )
    }

    /// How far ahead of the wall clock the backend was at the latest reading,
    /// or `None` before the first reading.
    pub fn offset(&self) -> Option<TimeDelta> {
        self.get_anchor().map(|a| a.backend_time - a.wall_clock)
    }

    /// Whether the wall clock currently agrees with the backend's time to
    /// within [`TRUST_TOLERANCE`]. False before the first backend reading.
    pub fn is_wall_clock_trusted(&self) -> bool {
        match self.trusted_now() {
            Some(trusted) => (trusted - self.raw_now()).abs() <= TRUST_TOLERANCE,
            None => false,
        }
    }

    fn get_anchor(&self) -> Option<Anchor> {
        *self.anchor.read().unwrap_or_else(|e| e.into_inner())
    }

    fn set_anchor(&self, anchor: Anchor) -> Option<Anchor> {
        let mut guard = self.anchor.write().unwrap_or_else(|e| e.into_inner());
        guard.replace(anchor)
    }
}
