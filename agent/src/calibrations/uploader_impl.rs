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
use chrono::{DateTime, TimeDelta, Utc};
use tracing::{error, info, warn};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct UploaderOptions {
    /// The most reports queued at once; intake is rejected beyond it.
    pub capacity: usize,
    /// The largest report content accepted, in bytes.
    pub max_report_bytes: u64,
    /// Total send attempts per report before it is dropped. Network errors
    /// don't count toward it.
    pub attempts: u32,
    /// Backoff between attempts; the exponent is the report's attempt count
    /// minus one.
    pub backoff: cooldown::Backoff,
    /// Backstop on the network-error exemption: a report received longer ago
    /// than this is dropped even if every failure so far was a network error.
    pub max_report_age: TimeDelta,
}

impl Default for UploaderOptions {
    /// At most 1,000 reports of up to 2 MiB each (2 GiB at worst). Miru
    /// accepts up to 1 MiB of calibration data, and 2 MiB leaves room for its
    /// JSON escaping. Retries match the data uploads: 30 attempts, 10 s
    /// doubling to 1 h, and a 7-day backstop.
    fn default() -> Self {
        Self {
            capacity: 1000,
            max_report_bytes: 2 * 1024 * 1024,
            attempts: 30,
            backoff: cooldown::Backoff {
                base_secs: 10,
                growth_factor: 2,
                max_secs: 3600,
            },
            max_report_age: TimeDelta::days(7),
        }
    }
}

/// What the queue holds for the worker to do next.
pub(crate) enum NextAttempt {
    /// This report is due for an attempt now.
    Ready(QueueEntry),
    /// The queue is empty.
    Empty,
    /// Every queued report is waiting out its backoff; the earliest is due
    /// after this wait.
    Backoff(Duration),
}

/// Single-threaded calibration uploader state and logic: the queue of report
/// metadata and the directory of report files. [`Uploader`] runs it inside an
/// actor and does the sending and waiting.
///
/// Each queued report's content is its own file, `<reports>/<entry id>.json`,
/// so large content is never rewritten with the queue. A failed report is
/// retried at the tail of the queue after its own backoff, as in
/// `data_uploads::upload`, so one failing report never blocks the others.
///
/// [`Uploader`]: crate::calibrations::Uploader
pub struct UploaderImpl<N> {
    queue: Queue,
    reports: Dir,
    options: UploaderOptions,
    now_fn: N,
}

impl<N> UploaderImpl<N>
where
    N: Fn() -> DateTime<Utc> + Send + Sync + 'static,
{
    /// Without a `snapshot_file` the queue lives in memory, so reports still
    /// queued at a restart are lost.
    pub(crate) fn new(
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

    /// The report to attempt now, or how long until one is due.
    pub(crate) async fn next_attempt(&mut self) -> NextAttempt {
        let now = (self.now_fn)();
        if let Some(entry) = self.queue.next_ready(now) {
            return NextAttempt::Ready(entry);
        }
        if self.queue.is_empty() {
            return NextAttempt::Empty;
        }
        // a deadline beyond the longest backoff came from a clock jump
        let horizon = now + TimeDelta::seconds(self.options.backoff.max_secs.max(0));
        self.queue.reset_invalid_deadlines(horizon).await;
        let wait = self
            .queue
            .earliest_next_attempt()
            .map_or(Duration::ZERO, |at| {
                (at - now).to_std().unwrap_or(Duration::ZERO)
            });
        NextAttempt::Backoff(wait)
    }

    /// Make every waiting report due now, once Miru is reachable again.
    pub(crate) async fn retry_now(&mut self) {
        let now = (self.now_fn)();
        self.queue.reset_invalid_deadlines(now).await;
    }

    /// The report to send for `entry`. `Ok(None)` when its file is gone: the
    /// entry is dropped, since there is nothing left to send.
    pub(crate) async fn read(
        &mut self,
        entry: &QueueEntry,
    ) -> Result<Option<Report>, CalibrationsErr> {
        let id = entry.id;
        let metadata = entry.job.clone();
        match files::read_json(&self.report_file(id)).await {
            Ok(content) => Ok(Some(Report { metadata, content })),
            Err(FileSysErr::PathDoesNotExistErr(_)) => {
                error!("calibrations: dropping report {id}, whose file is missing");
                self.queue.ack(id).await;
                Ok(None)
            }
            Err(FileSysErr::ParseJSONErr(e)) => Err(unreadable(e)),
            Err(e) => Err(e.into()),
        }
    }

    /// Settle `entry` once its send (or read) ended in `result`: remove it
    /// when sent, drop it when it can never be sent or is out of attempts,
    /// otherwise retry it at the tail after its backoff.
    pub(crate) async fn finish(&mut self, entry: QueueEntry, result: Result<(), CalibrationsErr>) {
        match result {
            Ok(()) => {
                self.remove(entry.id).await;
                info!("calibrations: uploaded report {}", entry.id);
            }
            Err(err) if err.is_network_conn_err() => self.network_failure(entry, err).await,
            Err(err) => self.counted_failure(entry, err).await,
        }
    }

    /// A network error doesn't count as an attempt. The report waits a flat
    /// `base_secs`, unless it has aged past the backstop.
    async fn network_failure(&mut self, entry: QueueEntry, err: CalibrationsErr) {
        let age = (self.now_fn)() - entry.job.calibrated_at;
        if age >= self.options.max_report_age {
            let days = age.num_days();
            error!(
                "calibrations: dropping report {}, unsent after {days} days of network errors: {err}",
                entry.id
            );
            return self.remove(entry.id).await;
        }
        warn!(
            "calibrations: failed to reach Miru with report {}; will retry: {err}",
            entry.id
        );
        self.retry_after(&entry, entry.attempts, self.options.backoff.base_secs)
            .await;
    }

    /// Any other failure counts as an attempt. The report is dropped when the
    /// error is terminal or it is out of attempts, and otherwise waits out
    /// its growing backoff.
    async fn counted_failure(&mut self, entry: QueueEntry, err: CalibrationsErr) {
        let id = entry.id;
        let attempts = entry.attempts.saturating_add(1);
        if err.is_terminal() {
            error!("calibrations: dropping report {id}, which can never be uploaded: {err}");
            return self.remove(id).await;
        }
        if attempts >= self.options.attempts {
            error!("calibrations: dropping report {id} after {attempts} attempts: {err}");
            return self.remove(id).await;
        }
        warn!("calibrations: attempt {attempts} to upload report {id} failed; will retry: {err}");
        let wait = cooldown::calc(&self.options.backoff, attempts - 1);
        self.retry_after(&entry, attempts, wait).await;
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
