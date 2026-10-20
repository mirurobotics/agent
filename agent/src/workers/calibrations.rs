// standard crates
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

// internal crates
use crate::buffer::BufferErr;
use crate::calibrations::{CalibrationsErr, InvalidReportErr, Sender, SharedBuffer};
use crate::cooldown;
use crate::errors::Error;
use crate::filesys::{errors::ParseJSONErr, FileSysErr};
use crate::sync::{syncer::SyncEvent, SyncerExt};
use crate::trace;
use crate::workers::next_sync_event;

// external crates
use chrono::Utc;
use tokio::sync::watch;
use tracing::{error, info, warn};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Options {
    pub backoff: cooldown::Backoff,
}

impl Default for Options {
    /// The data-upload backoff (`UploaderOptions::default().backoff`): 10 s,
    /// doubling, capped at 1 h.
    fn default() -> Self {
        Self {
            backoff: cooldown::Backoff {
                base_secs: 10,
                growth_factor: 2,
                max_secs: 3600,
            },
        }
    }
}

/// How one attempt at the oldest buffered report ended.
enum Outcome {
    /// Sent, parked or dropped: the report is no longer buffered.
    Resolved,
    /// The report stays at the head of the buffer and is retried after a wait.
    Retry { network: bool },
}

/// The run of consecutive non-network failures, in memory only, so a restart
/// retries at once.
struct Streak(u32);

impl Streak {
    /// How long to wait after `outcome`. Network errors wait a flat
    /// `base_secs` and leave the streak alone; other failures grow it.
    fn next_wait(&mut self, outcome: &Outcome, backoff: &cooldown::Backoff) -> Option<Duration> {
        let secs = match outcome {
            Outcome::Resolved => {
                self.0 = 0;
                return None;
            }
            Outcome::Retry { network: true } => backoff.base_secs,
            Outcome::Retry { network: false } => {
                self.0 = self.0.saturating_add(1);
                cooldown::calc(backoff, self.0 - 1)
            }
        };
        Some(Duration::from_secs(secs.max(0) as u64))
    }
}

/// Upload buffered calibrations to Miru one at a time, oldest first, until
/// `shutdown_signal` resolves. A shutdown drops any in-flight send; a report
/// is removed only after Miru accepts it, so it is re-sent on the next boot.
pub async fn run<S, Y, F, Fut>(
    options: &Options,
    buffer: &SharedBuffer,
    sender: &S,
    syncer: &Y,
    sleep_fn: F,
    mut shutdown_signal: Pin<Box<impl Future<Output = ()> + Send + 'static>>,
) where
    S: Sender,
    Y: SyncerExt,
    F: Fn(Duration) -> Fut,
    Fut: Future<Output = ()> + Send,
{
    tokio::select! {
        _ = shutdown_signal.as_mut() => {
            info!("calibrations: upload worker shutdown complete");
        }
        _ = run_impl(options, buffer, sender, syncer, sleep_fn) => {}
    }
}

async fn run_impl<S, Y, F, Fut>(
    options: &Options,
    buffer: &SharedBuffer,
    sender: &S,
    syncer: &Y,
    sleep_fn: F,
) where
    S: Sender,
    Y: SyncerExt,
    F: Fn(Duration) -> Fut,
    Fut: Future<Output = ()> + Send,
{
    let mut events = match syncer.subscribe().await {
        Ok(subscriber) => Some(subscriber),
        Err(e) => {
            error!("calibrations: no syncer events, so no early retry on reconnect: {e}");
            None
        }
    };
    let mut streak = Streak(0);
    loop {
        match attempt_next(buffer, sender).await {
            Some(outcome) => {
                if let Some(wait) = streak.next_wait(&outcome, &options.backoff) {
                    back_off(wait, &sleep_fn, &mut events).await;
                }
            }
            None => idle(buffer, &mut events).await,
        }
    }
}

/// Try the oldest buffered report, if any. The buffer is locked only to read
/// the report and to resolve it, never across the send.
async fn attempt_next<S: Sender>(buffer: &SharedBuffer, sender: &S) -> Option<Outcome> {
    let (id, read) = {
        let guard = buffer.lock().await;
        let id = guard.next_ready(Utc::now())?.id;
        (id, guard.read(id).await)
    };
    let result = match read {
        Ok(report) => sender.send(&report).await,
        Err(BufferErr::FileSysErr(FileSysErr::PathDoesNotExistErr(_))) => {
            return Some(drop_missing(buffer, id).await);
        }
        Err(BufferErr::FileSysErr(FileSysErr::ParseJSONErr(e))) => Err(unreadable(e)),
        Err(e) => Err(e.into()),
    };
    Some(resolve(buffer, id, result).await)
}

/// A report file that cannot be parsed can never be sent.
fn unreadable(e: ParseJSONErr) -> CalibrationsErr {
    CalibrationsErr::InvalidReportErr(InvalidReportErr {
        reason: format!("the report file cannot be parsed: {e}"),
        trace: trace!(),
    })
}

/// A report whose file is gone has nothing left to send or park.
async fn drop_missing(buffer: &SharedBuffer, id: Uuid) -> Outcome {
    error!("calibrations: dropping report {id}, whose file is missing");
    buffer.lock().await.remove(id).await;
    Outcome::Resolved
}

/// Remove a sent report, park one that can never be sent, and keep any
/// other at the head of the buffer for a retry.
async fn resolve(buffer: &SharedBuffer, id: Uuid, result: Result<(), CalibrationsErr>) -> Outcome {
    let mut guard = buffer.lock().await;
    let err = match result {
        Ok(()) => {
            guard.remove(id).await;
            info!("calibrations: uploaded report {id}");
            return Outcome::Resolved;
        }
        Err(err) => err,
    };
    if !err.is_terminal() {
        warn!("calibrations: failed to upload report {id}; will retry: {err}");
        return Outcome::Retry {
            network: err.is_network_conn_err(),
        };
    }
    match guard.park(id, buffer.failed_dir()).await {
        Ok(_) => {
            error!("calibrations: report {id} can never be uploaded; parked as failed/{id}.json: {err}");
            Outcome::Resolved
        }
        Err(park_err) => {
            error!("calibrations: failed to park report {id} ({err}): {park_err}");
            Outcome::Retry { network: false }
        }
    }
}

/// Wait out `wait`, ending early when a sync succeeds (Miru is reachable
/// again). The sleep is never restarted, so intake can't extend a backoff.
async fn back_off<F, Fut>(
    wait: Duration,
    sleep_fn: &F,
    events: &mut Option<watch::Receiver<SyncEvent>>,
) where
    F: Fn(Duration) -> Fut,
    Fut: Future<Output = ()> + Send,
{
    let sleep = sleep_fn(wait);
    tokio::pin!(sleep);
    loop {
        tokio::select! {
            _ = &mut sleep => return,
            event = next_sync_event(events) => {
                if reconnected(event, events) {
                    return;
                }
            }
        }
    }
}

/// Wait for intake to buffer a report, or for a sync event.
async fn idle(buffer: &SharedBuffer, events: &mut Option<watch::Receiver<SyncEvent>>) {
    tokio::select! {
        _ = buffer.woken() => {}
        event = next_sync_event(events) => {
            reconnected(event, events);
        }
    }
}

/// Whether `event` is a successful sync. An ended event stream disables the
/// subscription.
fn reconnected(event: Option<SyncEvent>, events: &mut Option<watch::Receiver<SyncEvent>>) -> bool {
    match event {
        Some(SyncEvent::SyncSuccess) => true,
        Some(_) => false,
        None => {
            error!("calibrations: syncer event stream ended; no early retry on reconnect");
            *events = None;
            false
        }
    }
}
