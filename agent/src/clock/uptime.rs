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
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::warn;

/// How often the [`UptimeSaver`] task writes the total to disk. A crash loses
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
///
/// This is a cheap read-only handle: clone it for every part of the agent
/// that needs uptime. One [`UptimeSaver`] writes it to disk.
#[derive(Clone, Debug)]
pub struct Uptime {
    clock: Clock,
    /// The total saved on disk when this process loaded it.
    base: Duration,
    loaded_at: Instant,
}

impl Uptime {
    /// Count on from `saved`, the total of earlier runs.
    pub fn new(clock: Clock, saved: Duration) -> Self {
        let loaded_at = clock.monotonic_now();
        Self {
            clock,
            base: saved,
            loaded_at,
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
}

/// The one writer of the uptime file. Run it with [`UptimeSaver::spawn`].
pub struct UptimeSaver {
    uptime: Uptime,
    file: Option<UptimeFile>,
}

impl UptimeSaver {
    /// Resume from the total saved in `file`, or from zero without one.
    pub fn new(clock: Clock, file: Option<UptimeFile>) -> Self {
        let saved = file
            .as_ref()
            .map(|file| Duration::from_secs(file.read().secs))
            .unwrap_or_default();
        Self {
            uptime: Uptime::new(clock, saved),
            file,
        }
    }

    /// A handle to the uptime this saver writes.
    pub fn uptime(&self) -> Uptime {
        self.uptime.clone()
    }

    /// Write the total now. A failed write is logged; the next save tries
    /// again.
    pub async fn save(&mut self) {
        let secs = self.uptime.now().as_secs();
        let Some(file) = self.file.as_mut() else {
            return;
        };
        if let Err(err) = file.patch(UptimeState { secs }).await {
            warn!("uptime: failed to save uptime: {err}");
        }
    }

    /// Save every [`SAVE_INTERVAL`] in a task until shut down, and once more
    /// on shutdown so a clean restart loses no uptime.
    pub fn spawn(self) -> (UptimeSaverHandle, JoinHandle<()>) {
        let (sender, receiver) = mpsc::channel(1);
        let handle = tokio::spawn(self.run(receiver));
        (UptimeSaverHandle { shutdown: sender }, handle)
    }

    async fn run(mut self, mut shutdown: mpsc::Receiver<oneshot::Sender<()>>) {
        let mut ticks = tokio::time::interval(SAVE_INTERVAL);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // the first tick completes at once; nothing has run yet to save
        ticks.tick().await;
        loop {
            tokio::select! {
                _ = ticks.tick() => self.save().await,
                done = shutdown.recv() => {
                    self.save().await;
                    if let Some(done) = done {
                        let _ = done.send(());
                    }
                    return;
                }
            }
        }
    }
}

/// Stops a spawned [`UptimeSaver`].
#[derive(Clone, Debug)]
pub struct UptimeSaverHandle {
    shutdown: mpsc::Sender<oneshot::Sender<()>>,
}

impl UptimeSaverHandle {
    /// Save the uptime one last time and stop the task. Returns once the
    /// save is done, or at once if the task has already stopped.
    pub async fn shutdown(&self) {
        let (done, saved) = oneshot::channel();
        if self.shutdown.send(done).await.is_ok() {
            let _ = saved.await;
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
