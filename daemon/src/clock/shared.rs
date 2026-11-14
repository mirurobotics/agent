//! [`Clock`]: monotonic, server and system time from one shared server
//! reading.

// standard crates
use std::fmt;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

// external crates
use chrono::{DateTime, TimeDelta, Utc};
use tracing::{debug, warn};

/// How far the system clock may differ from the server's time and still count
/// as synced. The `Date` header has one-second resolution and the response
/// spends some time in flight, so an exact match is not expected.
pub const SYNC_TOLERANCE: TimeDelta = TimeDelta::seconds(60);

type DateTimeFn = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;
type InstantFn = Arc<dyn Fn() -> Instant + Send + Sync>;

/// One reading of the server's time, with what the agent's own clocks said
/// at the same moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Anchor {
    /// The monotonic instant the reading was taken.
    observed_at: Instant,
    /// The server's time at that moment.
    server_time: DateTime<Utc>,
    /// The robot's system clock when the reading was taken.
    system_time: DateTime<Utc>,
}

/// The agent's shared clock. Cheap to clone; clones share the server anchor.
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
    /// A clock backed by the system clock and the monotonic clock.
    pub fn new() -> Self {
        Self::from_sources(Arc::new(Utc::now), Arc::new(Instant::now))
    }

    /// A clock with injected system and monotonic sources, so tests can
    /// control both.
    pub fn from_sources(datetime_fn: DateTimeFn, instant_fn: InstantFn) -> Self {
        Self {
            datetime_fn,
            instant_fn,
            anchor: Arc::new(RwLock::new(None)),
        }
    }

    /// The system clock as-is. It may be decades off; use it only for logs.
    pub fn system_now(&self) -> DateTime<Utc> {
        (self.datetime_fn)()
    }

    /// The current monotonic instant.
    pub fn monotonic_now(&self) -> Instant {
        (self.instant_fn)()
    }

    /// Monotonic time since `earlier`, or zero if `earlier` is in the future.
    pub fn elapsed_since(&self, earlier: Instant) -> Duration {
        self.monotonic_now().saturating_duration_since(earlier)
    }

    /// Records a reading of the server's time taken now. Later readings
    /// replace earlier ones.
    pub fn observe_server_time(&self, server_time: DateTime<Utc>) {
        let anchor = Anchor {
            observed_at: self.monotonic_now(),
            server_time,
            system_time: self.system_now(),
        };
        let previous = self.set_anchor(anchor);
        let offset_secs = (server_time - anchor.system_time).num_seconds();
        if offset_secs.abs() > SYNC_TOLERANCE.num_seconds() && previous.is_none() {
            warn!(
                "clock: system clock differs from the server by {offset_secs}s; using server time"
            );
        } else {
            debug!("clock: server time {server_time}, offset {offset_secs}s");
        }
    }

    /// The server's time now, or `None` if the agent has not heard from the
    /// server in this process.
    pub fn server_now(&self) -> Option<DateTime<Utc>> {
        let anchor = self.get_anchor()?;
        let elapsed = self
            .monotonic_now()
            .saturating_duration_since(anchor.observed_at);
        let elapsed = TimeDelta::from_std(elapsed).unwrap_or(TimeDelta::MAX);
        Some(
            anchor
                .server_time
                .checked_add_signed(elapsed)
                .unwrap_or(DateTime::<Utc>::MAX_UTC),
        )
    }

    /// How far ahead of the system clock the server was at the latest reading,
    /// or `None` before the first reading.
    pub fn offset(&self) -> Option<TimeDelta> {
        self.get_anchor().map(|a| a.server_time - a.system_time)
    }

    /// Whether the system clock currently agrees with the server's time to
    /// within [`SYNC_TOLERANCE`]. False before the first server reading.
    pub fn is_system_clock_synced(&self) -> bool {
        match self.server_now() {
            Some(server_now) => (server_now - self.system_now()).abs() <= SYNC_TOLERANCE,
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
