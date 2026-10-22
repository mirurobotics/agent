// standard crates
use std::collections::HashSet;
use std::str::FromStr;
use std::time::Duration;

// internal crates
use crate::calibrations::{
    errors::{CalibrationTooLargeErr, CalibrationsErr, InvalidReportErr},
    report::{Queue, QueueEntry, QueueSnapshotFile, Report},
};
use crate::cooldown;
use crate::errors::Error;
use crate::filesys::{
    dirs, errors::ParseJSONErr, files, Dir, File, FileSysErr, PathExt, WriteOptions,
};
use crate::queue::Persistence;
use crate::trace;

// external crates
use chrono::Utc;
use tracing::{error, info, warn};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct UploaderOptions {
    /// The most reports queued at once; intake is rejected beyond it.
    pub capacity: usize,
    /// The largest report content accepted, in bytes.
    pub max_report_bytes: u64,
    pub backoff: cooldown::Backoff,
}

impl Default for UploaderOptions {
    /// At most 1,000 reports of up to 2 MiB each (2 GiB at worst). Miru
    /// accepts up to 1 MiB of calibration data, and 2 MiB leaves room for its
    /// JSON escaping. The backoff is the data-upload one: 10 s, doubling,
    /// capped at 1 h.
    fn default() -> Self {
        Self {
            capacity: 1000,
            max_report_bytes: 2 * 1024 * 1024,
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
    /// Sent or dropped: the report is no longer queued.
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

/// Single-threaded calibration uploader state and logic: the queue of report
/// metadata, the directory of report files and the retry streak.
/// [`Uploader`] runs it inside an actor and does the sending and waiting.
///
/// Each queued report's content is its own file, `<reports>/<entry id>.json`,
/// so large content is never rewritten with the queue.
///
/// [`Uploader`]: crate::calibrations::Uploader
pub struct UploaderImpl {
    queue: Queue,
    reports: Dir,
    options: UploaderOptions,
    streak: Streak,
}

impl UploaderImpl {
    /// Without a `snapshot_file` the queue lives in memory, so reports still
    /// queued at a restart are lost.
    pub(crate) fn new(
        snapshot_file: Option<QueueSnapshotFile>,
        reports: Dir,
        options: UploaderOptions,
    ) -> Self {
        let queue = match snapshot_file {
            Some(file) => Queue::from_snapshot(options.capacity, file, Persistence::Durable),
            None => Queue::new(options.capacity),
        };
        Self {
            queue,
            reports,
            options,
            streak: Streak(0),
        }
    }

    /// Delete report files with no queue entry, left behind by a crash
    /// between dequeuing a report and deleting its file. Failures are logged
    /// and retried on the next boot.
    pub(crate) async fn sweep(&self) {
        if !self.reports.exists() {
            return;
        }
        let listing = match dirs::files(&self.reports).await {
            Ok(listing) => listing,
            Err(e) => {
                warn!("calibrations: failed to list report files: {e}");
                return;
            }
        };
        let queued: HashSet<Uuid> = self.queue.iter().map(|entry| entry.id).collect();
        for file in listing {
            if report_id(&file).is_some_and(|id| queued.contains(&id)) {
                continue;
            }
            warn!("calibrations: deleting report file {file}, which is not queued");
            delete(&file).await;
        }
    }

    /// Queue `report` and return its id. The queue entry is written before
    /// the report file, so a crash between the two leaves an entry without a
    /// file, which [`Self::read`] drops. On error nothing is queued.
    pub(crate) async fn enqueue(&mut self, report: Report) -> Result<Uuid, CalibrationsErr> {
        let bytes = report.content.to_string().into_bytes();
        self.verify_size(bytes.len() as u64)?;
        let id = self.queue.enqueue(report.metadata).await?;
        let file = self.report_file(id);
        if let Err(e) = files::write_bytes(&file, &bytes, WriteOptions::OVERWRITE_ATOMIC).await {
            self.queue.ack(id).await;
            return Err(e.into());
        }
        Ok(id)
    }

    pub(crate) fn len(&self) -> usize {
        self.queue.len()
    }

    /// The oldest report ready to send.
    pub(crate) fn next_ready(&self) -> Option<QueueEntry> {
        self.queue.next_ready(Utc::now())
    }

    /// The report to send for `entry`. `Ok(None)` when its file is gone: the
    /// entry is dropped, since there is nothing left to send.
    pub(crate) async fn read(
        &mut self,
        entry: QueueEntry,
    ) -> Result<Option<Report>, CalibrationsErr> {
        let id = entry.id;
        let metadata = entry.job;
        match files::read_json(&self.report_file(id)).await {
            Ok(content) => Ok(Some(Report { metadata, content })),
            Err(FileSysErr::PathDoesNotExistErr(_)) => {
                error!("calibrations: dropping report {id}, whose file is missing");
                self.queue.ack(id).await;
                self.streak
                    .next_wait(&Outcome::Resolved, &self.options.backoff);
                Ok(None)
            }
            Err(FileSysErr::ParseJSONErr(e)) => Err(unreadable(e)),
            Err(e) => Err(e.into()),
        }
    }

    /// Settle report `id` once its send (or read) ended in `result`, and
    /// return how long to wait before the next attempt. A sent report is
    /// removed, one that can never be sent is dropped, and any other stays at
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
                self.remove(id).await;
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
        self.remove(id).await;
        error!("calibrations: dropping report {id}, which can never be uploaded: {err}");
        Outcome::Resolved
    }

    /// Dequeue report `id`, then delete its file. The entry goes first, so a
    /// crash between the two leaves a file that [`Self::sweep`] deletes.
    async fn remove(&mut self, id: Uuid) {
        self.queue.ack(id).await;
        delete(&self.report_file(id)).await;
    }

    fn verify_size(&self, report_bytes: u64) -> Result<(), CalibrationTooLargeErr> {
        if report_bytes <= self.options.max_report_bytes {
            return Ok(());
        }
        let err = CalibrationTooLargeErr {
            report_bytes,
            max_bytes: self.options.max_report_bytes,
            trace: trace!(),
        };
        warn!("calibrations: rejecting report: {err}");
        Err(err)
    }

    fn report_file(&self, id: Uuid) -> File {
        self.reports.file(&format!("{}.json", id.hyphenated()))
    }
}

/// The report id a report file is named after, or `None` for any other file.
fn report_id(file: &File) -> Option<Uuid> {
    let name = file.name().ok()?;
    Uuid::from_str(name.strip_suffix(".json")?).ok()
}

/// Delete `file`, logging a failure. A file that is already gone is fine.
async fn delete(file: &File) {
    if let Err(e) = files::delete(file).await {
        warn!("calibrations: failed to delete report file {file}: {e}");
    }
}

/// A report file that cannot be parsed can never be sent.
fn unreadable(e: ParseJSONErr) -> CalibrationsErr {
    CalibrationsErr::InvalidReportErr(InvalidReportErr {
        reason: format!("the report file cannot be parsed: {e}"),
        trace: trace!(),
    })
}
