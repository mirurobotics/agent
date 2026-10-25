// standard crates
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

// internal crates
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
use crate::queue::{NextAttempt, Persistence, RetryPolicy, Verdict};

// external crates
use chrono::{DateTime, TimeDelta, Utc};
use tracing::{debug, error, info, warn};

/// A non-terminal failure after an attempt this long doesn't count toward the
/// attempt budget. Its likeliest cause is storage credentials that expired
/// mid-transfer, and the retry resumes the upload with fresh ones.
const LONG_ATTEMPT: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Debug)]
pub struct UploaderOptions {
    /// Maximum number of queued jobs. A job being uploaded stays queued until
    /// the backend confirms it, so it still counts against this.
    pub queue_capacity: usize,
    /// When a failed job is retried and when it is dropped. A job's age is
    /// measured from when its file was first observed.
    pub retry: RetryPolicy,
}

impl Default for UploaderOptions {
    fn default() -> Self {
        Self {
            queue_capacity: 1024,
            retry: RetryPolicy::default(),
        }
    }
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
            Some(file) => {
                Queue::from_snapshot(options.queue_capacity, file, Persistence::BestEffort)
            }
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
        self.queue.enqueue(job).await.map(drop).map_err(Into::into)
    }

    pub(crate) fn len(&self) -> usize {
        self.queue.len()
    }

    /// The entry to attempt now, or how long until one is eligible.
    pub(crate) async fn next_attempt(&mut self) -> NextAttempt<Job> {
        let now = (self.now_fn)();
        let horizon = self.options.retry.horizon(now);
        self.queue.next_attempt(now, horizon).await
    }

    /// Build one executor attempt on `entry`. The future is built from clones
    /// so it borrows nothing from `self`, leaving the worker free to serve
    /// commands while it runs.
    pub(crate) fn attempt(
        &self,
        entry: &QueueEntry,
    ) -> impl Future<Output = Result<(), UploadErr>> {
        Self::log_attempt(entry);
        let executor = self.executor.clone();
        let job = entry.job.clone();
        async move { executor.upload(&job).await }
    }

    /// Settle `entry` with the result of its attempt, which ran for `ran_for`:
    /// ack it on success or when it is dropped, otherwise retry it at the tail.
    pub(crate) async fn finish_attempt(
        &mut self,
        mut entry: QueueEntry,
        result: Result<(), UploadErr>,
        ran_for: Duration,
    ) {
        match result {
            Ok(()) => {
                entry.attempts += 1;
                Self::log_success(&entry);
                self.enqueue_delete_job(&entry).await;
                self.queue.ack(entry.id).await;
            }
            Err(err) => self.handle_failure(entry, err, ran_for).await,
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

    /// Settle a failed attempt as the retry policy says. Network connection
    /// errors, and non-terminal failures after a [`LONG_ATTEMPT`], don't count
    /// toward the attempt budget; any other failure does.
    async fn handle_failure(&mut self, mut entry: QueueEntry, err: UploadErr, ran_for: Duration) {
        let age = (self.now_fn)() - entry.job.first_observed_at;
        let retry = &self.options.retry;
        let long_attempt = ran_for >= LONG_ATTEMPT && !err.is_terminal();
        let verdict = if long_attempt {
            retry.on_uncounted_failure(entry.attempts, age)
        } else {
            retry.on_failure(&err, entry.attempts, age)
        };
        match verdict {
            Verdict::Retry {
                attempts,
                wait_secs,
            } => {
                entry.attempts = attempts;
                Self::log_failure(&entry, &err, long_attempt);
                self.retry_after(&entry, wait_secs).await;
            }
            Verdict::Terminal { attempts } => {
                entry.attempts = attempts;
                Self::log_failure(&entry, &err, long_attempt);
                Self::log_terminal_drop(&entry, &err);
                self.queue.ack(entry.id).await;
            }
            Verdict::OutOfAttempts { attempts } => {
                entry.attempts = attempts;
                Self::log_failure(&entry, &err, long_attempt);
                Self::log_dropped(&entry, &err);
                self.queue.ack(entry.id).await;
            }
            Verdict::TooOld { attempts, age } => {
                entry.attempts = attempts;
                Self::log_failure(&entry, &err, long_attempt);
                Self::log_age_drop(&entry, age, &err);
                self.queue.ack(entry.id).await;
            }
        }
    }

    /// Retry `entry` at the tail once `wait_secs` have passed, recording its
    /// attempt count.
    async fn retry_after(&mut self, entry: &QueueEntry, wait_secs: i64) {
        let file = &entry.job.file;
        let attempts = entry.attempts;
        info!(
            "upload: requeuing {file} at tail after {attempts} attempt(s), retrying in \
             {wait_secs}s"
        );
        let next_attempt_at = (self.now_fn)() + TimeDelta::seconds(wait_secs);
        self.queue.retry(entry.id, attempts, next_attempt_at).await;
    }

    /// A network error logs at debug, since it repeats every `base_secs` while
    /// the device is offline. A failure after a long attempt logs at info,
    /// since a long upload outlives its credentials. Any other failure counts
    /// and logs at warn.
    fn log_failure(entry: &QueueEntry, err: &UploadErr, long_attempt: bool) {
        if err.is_network_conn_err() {
            Self::log_network_conn_err(entry, err);
        } else if long_attempt {
            Self::log_long_attempt(entry, err);
        } else {
            Self::log_counted_failure(entry, err);
        }
    }

    fn log_network_conn_err(entry: &QueueEntry, err: &UploadErr) {
        debug!(
            "upload: network connection error for file {}; not counting attempt: {err:?}",
            entry.job.file
        );
    }

    fn log_long_attempt(entry: &QueueEntry, err: &UploadErr) {
        info!(
            "upload: long attempt for file {} failed; not counting it: {err:?}",
            entry.job.file
        );
    }

    fn log_counted_failure(entry: &QueueEntry, err: &UploadErr) {
        warn!(
            "upload: attempt {} for file {} failed: {err:?}",
            entry.attempts, entry.job.file
        );
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
            "dropping upload job: still failing {} days after the file was first observed \
             (rule {}, file {}, digest {}, attempt {}): {err:?}",
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
