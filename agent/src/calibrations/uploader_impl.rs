// standard crates
use std::time::Duration;

// internal crates
use crate::calibrations::{
    errors::{CalibrationsErr, InvalidReportErr},
    queue::{DirQueue, Report},
};
use crate::cooldown;
use crate::errors::Error;
use crate::filesys::{errors::ParseJSONErr, Dir, FileSysErr};
use crate::queue::DirQueueErr;
use crate::trace;

// external crates
use chrono::Utc;
use tracing::{error, info, warn};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct UploaderOptions {
    pub backoff: cooldown::Backoff,
}

impl Default for UploaderOptions {
    /// The data-upload backoff (`upload::UploaderOptions::default().backoff`):
    /// 10 s, doubling, capped at 1 h.
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

/// How one attempt at the oldest queued report ended.
enum Outcome {
    /// Sent, parked or dropped: the report is no longer queued.
    Resolved,
    /// The report stays at the head of the queue and is retried after a wait.
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

/// Single-threaded calibration uploader state and logic: the queue, where
/// rejected reports are parked, and the retry streak. [`Uploader`] runs it
/// inside an actor and does the sending and waiting.
///
/// [`Uploader`]: crate::calibrations::Uploader
pub struct UploaderImpl {
    queue: DirQueue,
    failed: Dir,
    options: UploaderOptions,
    streak: Streak,
}

impl UploaderImpl {
    pub(crate) fn new(queue: DirQueue, failed: Dir, options: UploaderOptions) -> Self {
        Self {
            queue,
            failed,
            options,
            streak: Streak(0),
        }
    }

    pub(crate) async fn enqueue(&mut self, report: Report) -> Result<Uuid, CalibrationsErr> {
        Ok(self.queue.enqueue(report).await?)
    }

    pub(crate) fn len(&self) -> usize {
        self.queue.len()
    }

    /// The id of the oldest report ready to send.
    pub(crate) fn next_ready(&self) -> Option<Uuid> {
        self.queue.next_ready(Utc::now()).map(|entry| entry.id)
    }

    /// The report to send for `id`. `Ok(None)` when its file is gone: the
    /// entry is dropped, since there is nothing left to send or park.
    pub(crate) async fn read(&mut self, id: Uuid) -> Result<Option<Report>, CalibrationsErr> {
        match self.queue.read(id).await {
            Ok(report) => Ok(Some(report)),
            Err(DirQueueErr::FileSysErr(FileSysErr::PathDoesNotExistErr(_))) => {
                error!("calibrations: dropping report {id}, whose file is missing");
                self.queue.remove(id).await;
                self.streak
                    .next_wait(&Outcome::Resolved, &self.options.backoff);
                Ok(None)
            }
            Err(DirQueueErr::FileSysErr(FileSysErr::ParseJSONErr(e))) => Err(unreadable(e)),
            Err(e) => Err(e.into()),
        }
    }

    /// Settle report `id` once its send (or read) ended in `result`, and
    /// return how long to wait before the next attempt. A sent report is
    /// removed, one that can never be sent is parked, and any other stays at
    /// the head of the queue for a retry.
    pub(crate) async fn finish(
        &mut self,
        id: Uuid,
        result: Result<(), CalibrationsErr>,
    ) -> Option<Duration> {
        let outcome = self.resolve(id, result).await;
        self.streak.next_wait(&outcome, &self.options.backoff)
    }

    async fn resolve(&mut self, id: Uuid, result: Result<(), CalibrationsErr>) -> Outcome {
        let err = match result {
            Ok(()) => {
                self.queue.remove(id).await;
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
        match self.queue.park(id, &self.failed).await {
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
}

/// A report file that cannot be parsed can never be sent.
fn unreadable(e: ParseJSONErr) -> CalibrationsErr {
    CalibrationsErr::InvalidReportErr(InvalidReportErr {
        reason: format!("the report file cannot be parsed: {e}"),
        trace: trace!(),
    })
}
