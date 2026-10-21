// standard crates
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::cooldown;
use crate::data_uploads::{
    retention::{DeleterExt, Job as DeleteJob},
    upload::{
        errors::*,
        executor::UploadExecutor,
        job::Job,
        queue::{Queue, QueueEntry, QueueSnapshotFile},
    },
};
use crate::errors::Error;
use crate::trace;

// external crates
use chrono::{DateTime, TimeDelta, Utc};
use tracing::{debug, error, info, warn};

#[derive(Clone, Debug)]
pub struct UploaderOptions {
    /// Maximum number of queued jobs. A job being uploaded stays queued until
    /// the backend confirms it, so it still counts against this.
    pub queue_capacity: usize,
    /// Total executor attempts per job before it is dropped.
    pub attempts: u32,
    /// Backoff between attempts; the exponent is the job's lifetime attempt
    /// count minus one, so waits grow across requeues and cap at `max_secs`.
    pub backoff: cooldown::Backoff,
    /// Fixed floor of the per-attempt upload deadline. Covers control-plane
    /// RPCs (create/confirm, each bounded at 3 × 10s attempts) and connection
    /// setup so tiny files are never starved.
    pub attempt_timeout_floor: Duration,
    /// Minimum-throughput assumption scaling the per-attempt deadline with
    /// file size. Deliberately far below any plausible sustained uplink: a
    /// false timeout discards transfer progress, so this errs generous while
    /// still guaranteeing every attempt terminates.
    pub attempt_timeout_bytes_per_sec: u64,
    /// Backstop on the network-error attempt exemption: a job first observed
    /// longer ago than this is dropped even if every failure so far was
    /// network-classified.
    pub max_job_age: TimeDelta,
}

impl Default for UploaderOptions {
    fn default() -> Self {
        Self {
            queue_capacity: 1024,
            attempts: 30,
            backoff: cooldown::Backoff {
                base_secs: 10,
                growth_factor: 2,
                max_secs: 3600,
            },
            attempt_timeout_floor: Duration::from_secs(120),
            attempt_timeout_bytes_per_sec: 64 * 1024,
            max_job_age: TimeDelta::days(7),
        }
    }
}

impl UploaderOptions {
    /// Deadline for one upload attempt of `size` bytes: floor plus a
    /// per-byte allowance at the minimum-throughput assumption.
    pub fn attempt_deadline(&self, size: u64) -> Duration {
        let bps = self.attempt_timeout_bytes_per_sec.max(1);
        self.attempt_timeout_floor
            .saturating_add(Duration::from_secs(size.div_ceil(bps)))
    }
}

/// What the queue holds for the worker to do next.
pub(crate) enum NextAttempt {
    /// This entry is eligible for an attempt now.
    Ready(QueueEntry),
    /// The queue is empty.
    Empty,
    /// Every queued entry is waiting out its backoff; the earliest becomes
    /// eligible after this wait.
    Backoff(Duration),
}

/// Single-threaded uploader state and logic. [`Uploader`] runs it inside an
/// actor so callers can share it across tasks.
///
/// [`Uploader`]: crate::data_uploads::upload::Uploader
pub struct UploaderImpl<ExecutorT, D, N>
where
    ExecutorT: UploadExecutor,
    D: DeleterExt + 'static,
    N: Fn() -> DateTime<Utc> + Send + Sync + 'static,
{
    queue: Queue,
    executor: Arc<ExecutorT>,
    deleter: Arc<D>,
    options: UploaderOptions,
    now_fn: N,
}

impl<ExecutorT, D, N> UploaderImpl<ExecutorT, D, N>
where
    ExecutorT: UploadExecutor,
    D: DeleterExt + 'static,
    N: Fn() -> DateTime<Utc> + Send + Sync + 'static,
{
    pub(crate) fn new(
        executor: Arc<ExecutorT>,
        deleter: Arc<D>,
        options: UploaderOptions,
        snapshot_file: Option<QueueSnapshotFile>,
        now_fn: N,
    ) -> Self {
        let queue = match snapshot_file {
            Some(file) => Queue::from_snapshot(options.queue_capacity, file),
            None => Queue::new(options.queue_capacity),
        };
        Self {
            queue,
            executor,
            deleter,
            options,
            now_fn,
        }
    }

    pub(crate) async fn enqueue(&mut self, job: Job) -> Result<(), UploadErr> {
        self.queue.enqueue(job).await?;
        Ok(())
    }

    pub(crate) fn len(&self) -> usize {
        self.queue.len()
    }

    /// The entry to attempt now, or how long until one is eligible.
    pub(crate) async fn next_attempt(&mut self) -> NextAttempt {
        let now = (self.now_fn)();
        if let Some(entry) = self.queue.next_ready(now) {
            return NextAttempt::Ready(entry);
        }
        if self.queue.is_empty() {
            return NextAttempt::Empty;
        }

        // every queued entry is waiting out its backoff: wait until the
        // earliest deadline
        self.queue
            .reset_invalid_deadlines(now + TimeDelta::seconds(self.options.backoff.max_secs.max(0)))
            .await;
        let wait = match self.queue.earliest_next_attempt() {
            Some(at) => (at - now).to_std().unwrap_or(Duration::ZERO),
            // unreachable: the queue is non-empty here
            None => Duration::ZERO,
        };
        NextAttempt::Backoff(wait)
    }

    /// Build one executor attempt on `entry`, bounded by its size-scaled
    /// deadline. The future is built from clones so it borrows nothing from
    /// `self`, leaving the worker free to serve commands while it runs.
    pub(crate) fn attempt(
        &self,
        entry: &QueueEntry,
    ) -> impl Future<Output = Result<(), UploadErr>> {
        Self::log_attempt(entry);
        let executor = self.executor.clone();
        let job = entry.job.clone();
        let deadline = self.options.attempt_deadline(job.size);
        async move {
            match tokio::time::timeout(deadline, executor.upload(&job)).await {
                Ok(result) => result,
                Err(_) => Err(UploadErr::AttemptTimeoutErr(AttemptTimeoutErr {
                    file: job.file.to_string(),
                    size: job.size,
                    deadline,
                    trace: trace!(),
                })),
            }
        }
    }

    /// Settle `entry` with the result of its attempt: ack it on success or
    /// when it is dropped, otherwise retry it at the tail.
    pub(crate) async fn finish_attempt(
        &mut self,
        mut entry: QueueEntry,
        result: Result<(), UploadErr>,
    ) {
        match result {
            Ok(()) => {
                entry.attempts += 1;
                Self::log_success(&entry);
                self.enqueue_delete_job(&entry).await;
                self.queue.ack(entry.id).await;
            }
            Err(err) if err.is_network_conn_err() => self.handle_network_failure(entry, err).await,
            Err(err) => self.handle_counted_failure(entry, err).await,
        }
    }

    /// Enqueues the delete job for a confirmed upload whose retention requires
    /// the upload. Retention that does not require it was already enqueued at
    /// stability by the retention sink, so enqueueing here too would book a
    /// second delete-queue slot and hash the file twice.
    async fn enqueue_delete_job(&mut self, entry: &QueueEntry) {
        let Some(retention) = &entry.job.retention else {
            return;
        };
        if !retention.requires_upload() {
            return;
        }

        let job = &entry.job;
        let delete_job = DeleteJob {
            file: job.file.clone(),
            size: job.size,
            digest: job.digest.clone(),
            mtime: job.mtime,
            first_observed_at: job.first_observed_at,
            last_observed_at: (self.now_fn)(),
            ttl_secs: retention.ttl_secs,
            file_rule_id: job.file_rule_id.clone(),
            deployment_id: job.deployment_id.clone(),
        };
        if let Err(e) = self.deleter.enqueue(delete_job).await {
            warn!(
                "upload: confirmed {} but failed to enqueue its delete job: {e:?}",
                job.file
            );
        }
    }

    /// Network connection errors are expected and do not count toward the
    /// attempt budget. Drop the job if it has aged past the backstop;
    /// otherwise stamp a flat cooldown and requeue at the tail.
    async fn handle_network_failure(&mut self, entry: QueueEntry, err: UploadErr) {
        let age = (self.now_fn)() - entry.job.first_observed_at;
        if age >= self.options.max_job_age {
            Self::log_age_drop(&entry, age, &err);
            self.queue.ack(entry.id).await;
            return;
        }

        let wait = self.options.backoff.base_secs.max(0);
        debug!(
            "upload: network connection error for file {}; not counting attempt, \
             retrying in {wait}s: {err:?}",
            entry.job.file
        );
        self.retry_after(&entry, wait).await;
    }

    /// Counted failures bump the attempt budget. Drop on a terminal error or
    /// when the budget is exhausted; otherwise stamp the growing backoff and
    /// requeue at the tail.
    async fn handle_counted_failure(&mut self, mut entry: QueueEntry, err: UploadErr) {
        entry.attempts += 1;
        warn!(
            "upload: attempt {} for file {} failed: {err:?}",
            entry.attempts, entry.job.file
        );

        if err.is_terminal() {
            Self::log_terminal_drop(&entry, &err);
            self.queue.ack(entry.id).await;
            return;
        }
        if entry.attempts >= self.options.attempts {
            Self::log_dropped(&entry, &err);
            self.queue.ack(entry.id).await;
            return;
        }

        let wait = cooldown::calc(&self.options.backoff, entry.attempts - 1).max(0);
        info!("upload: retrying {} in {wait}s", entry.job.file);
        self.retry_after(&entry, wait).await;
    }

    /// Retry `entry` at the tail once `wait_secs` have passed, recording its
    /// attempt count.
    async fn retry_after(&mut self, entry: &QueueEntry, wait_secs: i64) {
        let file = &entry.job.file;
        let attempts = entry.attempts;
        info!("upload: requeuing {file} at tail after {attempts} attempt(s)");
        let next_attempt_at = (self.now_fn)() + TimeDelta::seconds(wait_secs);
        self.queue.retry(entry.id, attempts, next_attempt_at).await;
    }

    fn log_attempt(entry: &QueueEntry) {
        let file = &entry.job.file;
        let rule = &entry.job.file_rule_id;
        let size = entry.job.size;
        let attempt = entry.attempts + 1;
        info!("upload: attempting {file} rule {rule} size {size} attempt {attempt}");
    }

    fn log_success(entry: &QueueEntry) {
        info!(
            "uploaded file {} (rule {}, digest {}) on attempt {}",
            entry.job.file, entry.job.file_rule_id, entry.job.digest, entry.attempts
        );
    }

    fn log_dropped(entry: &QueueEntry, err: &UploadErr) {
        error!(
            "dropping upload job after {} attempts (rule {}, file {}, digest {}): {err:?}",
            entry.attempts, entry.job.file_rule_id, entry.job.file, entry.job.digest
        );
    }

    fn log_age_drop(entry: &QueueEntry, age: TimeDelta, err: &UploadErr) {
        error!(
            "dropping upload job: network-classified failures only, for {} days since the file \
             was first observed (rule {}, file {}, digest {}, attempt {}); suspect a permanent \
             failure misclassified as a network error: {err:?}",
            age.num_days(),
            entry.job.file_rule_id,
            entry.job.file,
            entry.job.digest,
            entry.attempts
        );
    }

    fn log_terminal_drop(entry: &QueueEntry, err: &UploadErr) {
        error!(
            "dropping upload job: backend rejected the upload attempt \
             (rule {}, file {}, digest {}, attempt {}); the backend will not learn this \
             upload died: {err:?}",
            entry.job.file_rule_id, entry.job.file, entry.job.digest, entry.attempts
        );
    }
}
