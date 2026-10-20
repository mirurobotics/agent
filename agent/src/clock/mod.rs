//! Time sources that do not depend on the robot's wall clock being right.
//!
//! Robots without a battery-backed clock boot at 1970 (or at a stale saved
//! time) and only learn the real time once NTP syncs, if it ever does. The
//! agent therefore answers three different questions about time:
//!
//! - **How long has it been?** [`Clock::instant`] and [`Clock::elapsed_since`]
//!   use the monotonic clock, which never jumps but resets on reboot. Use these
//!   for cooldowns, backoff and refresh scheduling.
//! - **What time is it, for sure?** [`Clock::trusted_now`] is the backend's
//!   time, learned from the `Date` header of the latest backend response and
//!   advanced by monotonic time since. It is `None` until the agent has heard
//!   from the backend in this process.
//! - **Roughly what time is it?** [`Clock::raw_now`] is the wall clock as-is.
//!   Use it only for logs.

// standard crates
use std::fmt;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

// external crates
use chrono::{DateTime, TimeDelta, Utc};
use tracing::{debug, warn};

/// How far the wall clock may differ from the backend's time and still count
/// as trusted. The `Date` header has one-second resolution and the response
/// spends some time in flight, so an exact match is not expected.
pub const TRUST_TOLERANCE: TimeDelta = TimeDelta::seconds(60);

type RawFn = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;
type MonoFn = Arc<dyn Fn() -> Instant + Send + Sync>;

/// A reading of the backend's time paired with the monotonic instant it was
/// taken at, plus the wall clock at that moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Anchor {
    server: DateTime<Utc>,
    at: Instant,
    raw: DateTime<Utc>,
}

/// The agent's shared clock. Cheap to clone; clones share the backend anchor.
#[derive(Clone)]
pub struct Clock {
    raw_fn: RawFn,
    mono_fn: MonoFn,
    anchor: Arc<RwLock<Option<Anchor>>>,
}

impl fmt::Debug for Clock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Clock")
            .field("anchor", &self.read_anchor())
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
        Self::with_sources(Arc::new(Utc::now), Arc::new(Instant::now))
    }

    /// A clock with injected wall-clock and monotonic sources, so tests can
    /// control both.
    pub fn with_sources(raw_fn: RawFn, mono_fn: MonoFn) -> Self {
        Self {
            raw_fn,
            mono_fn,
            anchor: Arc::new(RwLock::new(None)),
        }
    }

    /// The wall clock as-is. It may be decades off; use it only for logs.
    pub fn raw_now(&self) -> DateTime<Utc> {
        (self.raw_fn)()
    }

    /// The current monotonic instant.
    pub fn instant(&self) -> Instant {
        (self.mono_fn)()
    }

    /// Monotonic time since `earlier`, or zero if `earlier` is in the future.
    pub fn elapsed_since(&self, earlier: Instant) -> Duration {
        self.instant().saturating_duration_since(earlier)
    }

    /// Records a reading of the backend's time taken now. Later readings
    /// replace earlier ones.
    pub fn observe_server_time(&self, server: DateTime<Utc>) {
        let anchor = Anchor {
            server,
            at: self.instant(),
            raw: self.raw_now(),
        };
        let previous = self.write_anchor(anchor);
        // computed up front: arguments of a disabled log macro never run
        let offset_secs = (server - anchor.raw).num_seconds();
        if offset_secs.abs() > TRUST_TOLERANCE.num_seconds() && previous.is_none() {
            warn!(
                "clock: wall clock differs from the backend by {offset_secs}s; using backend time"
            );
        } else {
            debug!("clock: backend time {server}, offset {offset_secs}s");
        }
    }

    /// Records the backend's time from an HTTP `Date` header value. Values
    /// that do not parse are ignored.
    pub fn observe_date_header(&self, value: &str) {
        match parse_http_date(value) {
            Some(server) => self.observe_server_time(server),
            None => debug!("clock: ignoring unparseable Date header {value:?}"),
        }
    }

    /// The backend's time now, or `None` if the agent has not heard from the
    /// backend in this process.
    pub fn trusted_now(&self) -> Option<DateTime<Utc>> {
        let anchor = self.read_anchor()?;
        let elapsed = self.instant().saturating_duration_since(anchor.at);
        let elapsed = TimeDelta::from_std(elapsed).unwrap_or(TimeDelta::MAX);
        Some(
            anchor
                .server
                .checked_add_signed(elapsed)
                .unwrap_or(DateTime::<Utc>::MAX_UTC),
        )
    }

    /// How far ahead of the wall clock the backend was at the latest reading,
    /// or `None` before the first reading.
    pub fn offset(&self) -> Option<TimeDelta> {
        self.read_anchor().map(|a| a.server - a.raw)
    }

    /// Whether the wall clock currently agrees with the backend's time to
    /// within [`TRUST_TOLERANCE`]. False before the first backend reading.
    pub fn is_wall_clock_trusted(&self) -> bool {
        match self.trusted_now() {
            Some(trusted) => (trusted - self.raw_now()).abs() <= TRUST_TOLERANCE,
            None => false,
        }
    }

    fn read_anchor(&self) -> Option<Anchor> {
        *self.anchor.read().unwrap_or_else(|e| e.into_inner())
    }

    fn write_anchor(&self, anchor: Anchor) -> Option<Anchor> {
        let mut guard = self.anchor.write().unwrap_or_else(|e| e.into_inner());
        guard.replace(anchor)
    }
}

/// Parses an HTTP `Date` header (IMF-fixdate, e.g. `Tue, 20 Oct 2026 02:58:04
/// GMT`).
pub fn parse_http_date(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc2822(value.trim())
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}
