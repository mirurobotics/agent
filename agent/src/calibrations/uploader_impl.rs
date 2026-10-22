// standard crates
use std::collections::HashSet;
use std::future::Future;
use std::str::FromStr;
use std::sync::Arc;

// internal crates
use crate::calibrations::{
    errors::{CalibrationTooLargeErr, CalibrationsErr, InvalidReportErr},
    report::{Queue, QueueEntry, QueueSnapshotFile, Report, ReportMetadata},
    sender::Sender,
};
use crate::errors::Error;
use crate::filesys::{dirs, files, Dir, File, FileSysErr, PathExt, WriteOptions};
use crate::queue::{self, Persistence, RetryPolicy, Verdict};
use crate::trace;

// external crates
use chrono::{DateTime, TimeDelta, Utc};
use tracing::{error, info, warn};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct UploaderOptions {
    /// The most reports queued at once; intake is rejected beyond it.
    pub capacity: usize,
    /// The largest report data accepted, in bytes.
    pub max_report_bytes: u64,
    /// When a failed report is retried and when it is dropped. Its age is
    /// measured from `calibrated_at`.
    pub retry: RetryPolicy,
}

impl Default for UploaderOptions {
    /// At most 1,000 reports of up to 1 MiB of data each (1 GiB at worst),
    /// the most calibration data Miru accepts, retried as the data uploads
    /// are.
    fn default() -> Self {
        Self {
            capacity: 1000,
            max_report_bytes: 1024 * 1024,
            retry: RetryPolicy::default(),
        }
    }
}

/// What the queue holds for the worker to do next.
pub(crate) type NextAttempt = queue::NextAttempt<ReportMetadata>;

/// Single-threaded calibration uploader state and logic: the queue of report
/// metadata, the directory of report files, and the sender that uploads them.
/// [`Uploader`] runs it inside an actor and does the waiting.
///
/// Each queued report's data is its own file, `<reports>/<entry id>`, so large
/// data is never rewritten with the queue. A failed report is
/// retried at the tail of the queue after its own backoff, as in
/// `data_uploads::upload`, so one failing report never blocks the others.
///
/// [`Uploader`]: crate::calibrations::Uploader
pub struct UploaderImpl<S, N> {
    sender: Arc<S>,
    queue: Queue,
    reports: Dir,
    options: UploaderOptions,
    now_fn: N,
}

impl<S, N> UploaderImpl<S, N>
where
    S: Sender + 'static,
    N: Fn() -> DateTime<Utc> + Send + Sync + 'static,
{
    /// Without a `snapshot_file` the queue lives in memory, so reports still
    /// queued at a restart are lost.
    pub(crate) fn new(
        sender: Arc<S>,
        snapshot_file: Option<QueueSnapshotFile>,
        reports: Dir,
        options: UploaderOptions,
        now_fn: N,
    ) -> Self {
        let queue = match snapshot_file {
            Some(file) => Queue::from_snapshot(options.capacity, file, Persistence::Durable),
            None => Queue::new(options.capacity),
        };
        Self {
            sender,
            queue,
            reports,
            options,
            now_fn,
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
    /// file, which [`Self::upload`] fails for good. On error nothing is queued.
    pub(crate) async fn enqueue(&mut self, report: Report) -> Result<Uuid, CalibrationsErr> {
        let bytes = report.data.into_bytes();
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

    /// The report to attempt now, or how long until one is due.
    pub(crate) async fn next_attempt(&mut self) -> NextAttempt {
        let now = (self.now_fn)();
        let horizon = self.options.retry.horizon(now);
        self.queue.next_attempt(now, horizon).await
    }

    /// Make every waiting report due now, once Miru is reachable again.
    pub(crate) async fn retry_now(&mut self) {
        let now = (self.now_fn)();
        self.queue.reset_invalid_deadlines(now).await;
    }

    /// Upload the report queued as `entry`: read its file, then send it. The
    /// future owns what it needs and borrows nothing from `self`, so the
    /// worker can serve commands while it runs.
    pub(crate) fn upload(
        &self,
        entry: &QueueEntry,
    ) -> impl Future<Output = Result<(), CalibrationsErr>> {
        let sender = self.sender.clone();
        let metadata = entry.job.clone();
        let file = self.report_file(entry.id);
        async move {
            let data = read(&file).await?;
            sender.send(Report { metadata, data }).await
        }
    }

    /// Settle `entry` once its upload ended in `result`: remove it
    /// when sent, otherwise retry or drop it as the retry policy says.
    pub(crate) async fn finish(&mut self, entry: QueueEntry, result: Result<(), CalibrationsErr>) {
        let err = match result {
            Ok(()) => {
                self.remove(entry.id).await;
                info!("calibrations: uploaded report {}", entry.id);
                return;
            }
            Err(err) => err,
        };
        let id = entry.id;
        let age = (self.now_fn)() - entry.job.calibrated_at;
        match self.options.retry.on_failure(&err, entry.attempts, age) {
            Verdict::Retry {
                attempts,
                wait_secs,
            } => {
                log_retry(id, attempts, &err);
                self.retry_after(&entry, attempts, wait_secs).await;
            }
            Verdict::Terminal { .. } => {
                error!("calibrations: dropping report {id}, which can never be uploaded: {err}");
                self.remove(id).await;
            }
            Verdict::OutOfAttempts { attempts } => {
                error!("calibrations: dropping report {id} after {attempts} attempts: {err}");
                self.remove(id).await;
            }
            Verdict::TooOld { age, .. } => {
                let days = age.num_days();
                error!("calibrations: dropping report {id}, still unsent after {days} days: {err}");
                self.remove(id).await;
            }
        }
    }

    /// Move `entry` to the tail, due again in `wait_secs`.
    async fn retry_after(&mut self, entry: &QueueEntry, attempts: u32, wait_secs: i64) {
        let next_attempt_at = (self.now_fn)() + TimeDelta::seconds(wait_secs.max(0));
        self.queue.retry(entry.id, attempts, next_attempt_at).await;
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
        self.reports.file(&id.hyphenated().to_string())
    }
}

/// A network error doesn't count as an attempt, so its retry log says so.
fn log_retry(id: Uuid, attempts: u32, err: &CalibrationsErr) {
    if err.is_network_conn_err() {
        warn!("calibrations: failed to reach Miru with report {id}; will retry: {err}");
        return;
    }
    warn!("calibrations: attempt {attempts} to upload report {id} failed; will retry: {err}");
}

/// The report id a report file is named after, or `None` for any other file.
fn report_id(file: &File) -> Option<Uuid> {
    Uuid::from_str(file.name().ok()?).ok()
}

/// Delete `file`, logging a failure. A file that is already gone is fine.
async fn delete(file: &File) {
    if let Err(e) = files::delete(file).await {
        warn!("calibrations: failed to delete report file {file}: {e}");
    }
}

/// The data in report `file`. A missing file or one that isn't UTF-8 can
/// never be sent, so either is a terminal [`InvalidReportErr`]; any other
/// read error is retried.
async fn read(file: &File) -> Result<String, CalibrationsErr> {
    match files::read_string(file).await {
        Ok(data) => Ok(data),
        Err(FileSysErr::PathDoesNotExistErr(_)) => Err(invalid("the report file is missing")),
        Err(FileSysErr::ConvertUTF8Err(e)) => {
            Err(invalid(&format!("the report file is not UTF-8: {e}")))
        }
        Err(e) => Err(e.into()),
    }
}

fn invalid(reason: &str) -> CalibrationsErr {
    CalibrationsErr::InvalidReportErr(InvalidReportErr {
        reason: reason.to_string(),
        trace: trace!(),
    })
}
