//! [`Uptime`]: how long the agent has run, summed across restarts.

// standard crates
use std::time::{Duration, Instant};

// internal crates
use crate::clock::Clock;
use crate::filesys::state_file::StateFile;
use crate::models::Patch;

// external crates
use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use tracing::warn;

/// How often [`Uptime::save_if_due`] writes the total to disk. A crash loses
/// at most this much uptime, which only makes deadlines later.
pub const SAVE_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// The saved total, in whole seconds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UptimeState {
    pub secs: u64,
}

impl Patch<UptimeState> for UptimeState {
    fn patch(&mut self, patch: UptimeState) {
        *self = patch;
    }
}

pub type UptimeFile = StateFile<UptimeState, UptimeState>;

/// Monotonic time the agent has been running, summed across restarts.
///
/// It never jumps, whatever the system clock does, and it never runs ahead of
/// real time: time the agent is not running does not count, and neither does
/// the unsaved time before a crash. So a deadline counted on it is reached no
/// sooner than the same span of real time.
pub struct Uptime {
    clock: Clock,
    /// The total saved on disk when this process loaded it.
    base: Duration,
    loaded_at: Instant,
    last_saved_at: Instant,
    file: Option<UptimeFile>,
}

impl Uptime {
    /// Resume from the total saved in `file`, or from zero without one.
    pub fn new(clock: Clock, file: Option<UptimeFile>) -> Self {
        let base = file
            .as_ref()
            .map(|file| Duration::from_secs(file.read().secs))
            .unwrap_or_default();
        let now = clock.monotonic_now();
        Self {
            clock,
            base,
            loaded_at: now,
            last_saved_at: now,
            file,
        }
    }

    /// The uptime so far.
    pub fn now(&self) -> Duration {
        self.base + self.clock.elapsed_since(self.loaded_at)
    }

    /// [`Uptime::now`] as a timestamp counted from the Unix epoch, for code
    /// that orders `DateTime`s. It is not a calendar date.
    pub fn now_as_datetime(&self) -> DateTime<Utc> {
        as_datetime(self.now())
    }

    /// Save the total if [`SAVE_INTERVAL`] has passed since the last save. A
    /// failed write is logged; the next call tries again.
    pub async fn save_if_due(&mut self) {
        if self.clock.elapsed_since(self.last_saved_at) < SAVE_INTERVAL {
            return;
        }
        let secs = self.now().as_secs();
        let Some(file) = self.file.as_mut() else {
            return;
        };
        match file.patch(UptimeState { secs }).await {
            Ok(()) => self.last_saved_at = self.clock.monotonic_now(),
            Err(err) => warn!("uptime: failed to save uptime: {err}"),
        }
    }
}

/// `uptime` as a timestamp counted from the Unix epoch, saturating to the far
/// future.
pub fn as_datetime(uptime: Duration) -> DateTime<Utc> {
    TimeDelta::from_std(uptime)
        .ok()
        .and_then(|delta| DateTime::UNIX_EPOCH.checked_add_signed(delta))
        .unwrap_or(DateTime::<Utc>::MAX_UTC)
}
