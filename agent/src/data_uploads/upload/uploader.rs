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
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

macro_rules! dispatch {
    ($op:expr, $respond_to:expr, $msg:expr) => {{
        let result = $op;
        if $respond_to.send(result).is_err() {
            error!($msg);
        }
    }};
}

#[derive(Clone, Debug)]
pub struct UploaderOptions {
    /// Maximum number of queued jobs. A job being uploaded stays queued until
    /// the backend confirms it, so it still counts against this. At capacity a
    /// new job is not refused: the queue evicts an existing entry (logged at
    /// `error!`) and admits the new one in its place.
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

// =================================== TRAIT ======================================= //
#[allow(async_fn_in_trait)]
// an async, actor-round-tripping is_empty would be dead weight next to len()
#[allow(clippy::len_without_is_empty)]
pub trait UploaderExt: Send + Sync {
    /// Push a job at the tail of the queue. The job is always admitted — at
    /// capacity the queue evicts an existing entry to make room — so the only
    /// errors are transport failures reaching the actor.
    async fn enqueue(&self, job: Job) -> Result<(), UploadErr>;
    /// The number of queued jobs, including one being uploaded — it stays
    /// queued until the backend confirms it.
    async fn len(&self) -> Result<usize, UploadErr>;
    /// Stop the actor, dropping any in-flight upload future (see the cancel
    /// safety contract on [`UploadExecutor`]). Queued jobs, including the
    /// interrupted one, stay on disk and are retried by the next process.
    async fn shutdown(&self) -> Result<(), UploadErr>;
}

// ================================== WORKER ======================================= //
pub(crate) enum Command {
    Enqueue {
        job: Job,
        respond_to: oneshot::Sender<()>,
    },
    Len {
        respond_to: oneshot::Sender<Result<usize, UploadErr>>,
    },
    Shutdown {
        respond_to: oneshot::Sender<Result<(), UploadErr>>,
    },
}

/// Whether the run loop should keep going after handling a command or job.
enum Flow {
    Continue,
    Shutdown,
}

/// Outcome of driving a single executor attempt to completion (or shutdown).
enum AttemptOutcome {
    /// The upload succeeded; the attempt is done.
    Succeeded,
    /// The upload failed; carries the error for logging and retry decisions.
    Failed(UploadErr),
    /// A shutdown arrived mid-attempt; the run loop must stop.
    ShuttingDown,
}

pub(crate) struct Worker<ExecutorT, D, F, Fut, N>
where
    ExecutorT: UploadExecutor,
    D: DeleterExt + 'static,
    F: Fn(Duration) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
    N: Fn() -> DateTime<Utc> + Send + Sync + 'static,
{
    receiver: Receiver<Command>,
    queue: Queue,
    executor: Arc<ExecutorT>,
    deleter: Arc<D>,
    options: UploaderOptions,
    sleep_fn: F,
    now_fn: N,
}

impl<ExecutorT, D, F, Fut, N> Worker<ExecutorT, D, F, Fut, N>
where
    ExecutorT: UploadExecutor,
    D: DeleterExt + 'static,
    F: Fn(Duration) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
    N: Fn() -> DateTime<Utc> + Send + Sync + 'static,
{
    pub(crate) async fn run(mut self) {
        loop {
            let now = (self.now_fn)();
            match self.queue.next_ready(now) {
                Some(entry) => {
                    if let Flow::Shutdown = self.run_attempt(entry).await {
                        break;
                    }
                }
                // idle: nothing to interleave, just wait for the next command
                None if self.queue.is_empty() => {
                    info!("upload: queue empty; awaiting next command");
                    match self.receiver.recv().await {
                        // all senders dropped
                        None => break,
                        Some(cmd) => {
                            if let Flow::Shutdown = self.handle_command(cmd).await {
                                break;
                            }
                        }
                    }
                }
                // every queued entry is waiting out its backoff: sleep until
                // the earliest deadline (or a command) and re-evaluate
                None => {
                    self.queue.reset_invalid_deadlines(
                        now + TimeDelta::seconds(self.options.backoff.max_secs.max(0)),
                    );
                    let wait = match self.queue.earliest_next_attempt() {
                        Some(at) => (at - now).to_std().unwrap_or(Duration::ZERO),
                        // unreachable: the queue is non-empty here
                        None => Duration::ZERO,
                    };
                    if let Flow::Shutdown = self.idle_wait(wait).await {
                        break;
                    }
                }
            }
        }
    }

    async fn run_attempt(&mut self, mut entry: QueueEntry) -> Flow {
        Self::log_attempt(&entry);
        match self.attempt_upload(&entry).await {
            // the attempt was cancelled -> leave the entry in the queue so the next
            // boot picks it up rather than losing it
            AttemptOutcome::ShuttingDown => Flow::Shutdown,
            AttemptOutcome::Succeeded => {
                entry.attempts += 1;
                Self::log_success(&entry);
                self.enqueue_delete_job(&entry).await;
                self.queue.remove(entry.id).await;
                Flow::Continue
            }
            AttemptOutcome::Failed(err) if err.is_network_conn_err() => {
                self.handle_network_failure(entry, err).await
            }
            AttemptOutcome::Failed(err) => self.handle_counted_failure(entry, err).await,
        }
    }

    async fn enqueue_delete_job(&mut self, entry: &QueueEntry) {
        let Some(retention) = &entry.job.retention else {
            return;
        };

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
    async fn handle_network_failure(&mut self, entry: QueueEntry, err: UploadErr) -> Flow {
        let age = (self.now_fn)() - entry.job.first_observed_at;
        if age >= self.options.max_job_age {
            Self::log_age_drop(&entry, age, &err);
            self.queue.remove(entry.id).await;
            return Flow::Continue;
        }

        let wait = self.options.backoff.base_secs.max(0);
        debug!(
            "upload: network connection error for file {}; not counting attempt, \
             retrying in {wait}s: {err:?}",
            entry.job.file
        );
        self.requeue_after(entry, wait).await;
        Flow::Continue
    }

    /// Counted failures bump the attempt budget. Drop on a terminal error or
    /// when the budget is exhausted; otherwise stamp the growing backoff and
    /// requeue at the tail.
    async fn handle_counted_failure(&mut self, mut entry: QueueEntry, err: UploadErr) -> Flow {
        entry.attempts += 1;
        warn!(
            "upload: attempt {} for file {} failed: {err:?}",
            entry.attempts, entry.job.file
        );

        if err.is_terminal() {
            Self::log_terminal_drop(&entry, &err);
            self.queue.remove(entry.id).await;
            return Flow::Continue;
        }
        if entry.attempts >= self.options.attempts {
            Self::log_dropped(&entry, &err);
            self.queue.remove(entry.id).await;
            return Flow::Continue;
        }

        let wait = cooldown::calc(&self.options.backoff, entry.attempts - 1).max(0);
        info!("upload: retrying {} in {wait}s", entry.job.file);
        self.requeue_after(entry, wait).await;
        Flow::Continue
    }

    /// Stamp `entry` eligible after `wait_secs` and append it at the tail.
    async fn requeue_after(&mut self, mut entry: QueueEntry, wait_secs: i64) {
        entry.next_attempt_at = Some((self.now_fn)() + TimeDelta::seconds(wait_secs));
        self.requeue(entry).await;
    }

    /// Sleep for `wait`, staying responsive to commands. Deliberately NOT
    /// [`Self::run_until_shutdown`]: that helper keeps driving its future after
    /// handling a command, but an enqueue here must return to the run loop
    /// immediately so a newly eligible entry is re-evaluated rather than
    /// waiting out the sleep.
    async fn idle_wait(&mut self, wait: Duration) -> Flow {
        let sleep_fut = (self.sleep_fn)(wait);
        tokio::select! {
            biased;
            cmd = self.receiver.recv() => match cmd {
                // all senders dropped
                None => Flow::Shutdown,
                Some(cmd) => self.handle_command(cmd).await,
            },
            () = sleep_fut => Flow::Continue,
        }
    }

    /// Drive one executor attempt on `entry` to completion while staying
    /// responsive to commands. The future is built from clones so it borrows
    /// nothing from `self`, leaving `self` free to serve commands while it runs.
    async fn attempt_upload(&mut self, entry: &QueueEntry) -> AttemptOutcome {
        let executor = self.executor.clone();
        let job = entry.job.clone();
        let deadline = self.options.attempt_deadline(job.size);
        let attempt = async move {
            match tokio::time::timeout(deadline, executor.upload(&job)).await {
                Ok(result) => result,
                Err(_) => Err(UploadErr::AttemptTimeoutErr(AttemptTimeoutErr {
                    file: job.file.to_string(),
                    size: job.size,
                    deadline,
                    trace: trace!(),
                })),
            }
        };
        match self.run_until_shutdown(attempt).await {
            None => AttemptOutcome::ShuttingDown,
            Some(Ok(())) => AttemptOutcome::Succeeded,
            Some(Err(err)) => AttemptOutcome::Failed(err),
        }
    }

    /// Requeue `entry` at the tail with its attempt count preserved. The queue
    /// refuses an entry it no longer holds: it was evicted to make room for a
    /// newer file while this attempt was in flight, and must stay evicted.
    async fn requeue(&mut self, entry: QueueEntry) {
        let file = entry.job.file.clone();
        let attempts = entry.attempts;
        info!("upload: requeuing {file} at tail after {attempts} attempt(s)");
        if !self.queue.requeue(entry).await {
            warn!(
                "upload: {file} was evicted from the queue while it was uploading; \
                 dropping its requeue after {attempts} attempt(s)"
            );
        }
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

    /// Run `fut` to completion while continuing to serve commands. Returns
    /// `None` when a shutdown arrived (or all senders dropped), in which case
    /// `fut` is dropped — cancelling an in-flight upload.
    async fn run_until_shutdown<T>(&mut self, fut: impl Future<Output = T>) -> Option<T> {
        tokio::pin!(fut);
        loop {
            tokio::select! {
                result = &mut fut => return Some(result),
                // recv is cancel-safe: losing the race never drops a command
                cmd = self.receiver.recv() => match cmd {
                    None => return None,
                    Some(cmd) => {
                        if let Flow::Shutdown = self.handle_command(cmd).await {
                            return None;
                        }
                    }
                }
            }
        }
    }

    async fn handle_command(&mut self, cmd: Command) -> Flow {
        match cmd {
            Command::Shutdown { respond_to } => {
                if respond_to.send(Ok(())).is_err() {
                    error!("Actor failed to send shutdown response");
                }
                Flow::Shutdown
            }
            Command::Enqueue { job, respond_to } => {
                let now = (self.now_fn)();
                dispatch!(
                    self.queue.enqueue(job, now).await,
                    respond_to,
                    "Actor failed to send enqueue response"
                );
                Flow::Continue
            }
            Command::Len { respond_to } => {
                dispatch!(
                    Ok(self.queue.len()),
                    respond_to,
                    "Actor failed to send len response"
                );
                Flow::Continue
            }
        }
    }
}

// ================================== HANDLE ======================================= //
#[derive(Debug)]
pub struct Uploader {
    sender: Sender<Command>,
}

impl Uploader {
    pub fn spawn<ExecutorT, D, F, Fut, N>(
        buffer_size: usize,
        executor: Arc<ExecutorT>,
        deleter: Arc<D>,
        options: UploaderOptions,
        snapshot_file: Option<QueueSnapshotFile>,
        sleep_fn: F,
        now_fn: N,
    ) -> Result<(Self, JoinHandle<()>), UploadErr>
    where
        ExecutorT: UploadExecutor + 'static,
        D: DeleterExt + 'static,
        F: Fn(Duration) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
        N: Fn() -> DateTime<Utc> + Send + Sync + 'static,
    {
        let (sender, receiver) = mpsc::channel(buffer_size);
        let queue = match snapshot_file {
            Some(file) => Queue::from_snapshot(options.queue_capacity, file),
            None => Queue::new(options.queue_capacity),
        };
        let worker = Worker {
            receiver,
            queue,
            executor,
            deleter,
            options,
            sleep_fn,
            now_fn,
        };
        let worker_handle = tokio::spawn(worker.run());
        Ok((Self { sender }, worker_handle))
    }

    async fn send_command<R>(
        &self,
        op: &str,
        make_cmd: impl FnOnce(oneshot::Sender<R>) -> Command,
    ) -> Result<R, UploadErr> {
        let (send, recv) = oneshot::channel();
        self.sender.send(make_cmd(send)).await.map_err(|e| {
            error!("Failed to send {op} command to actor: {e:?}");
            UploadErr::SendActorMessageErr(SendActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })?;
        recv.await.map_err(|e| {
            error!("Failed to receive {op} response from actor: {e:?}");
            UploadErr::ReceiveActorMessageErr(ReceiveActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })
    }
}

impl UploaderExt for Uploader {
    async fn enqueue(&self, job: Job) -> Result<(), UploadErr> {
        self.send_command("enqueue", |tx| Command::Enqueue {
            job,
            respond_to: tx,
        })
        .await
    }

    async fn len(&self) -> Result<usize, UploadErr> {
        self.send_command("len", |tx| Command::Len { respond_to: tx })
            .await?
    }

    async fn shutdown(&self) -> Result<(), UploadErr> {
        info!("Shutting down uploader...");
        self.send_command("shutdown", |tx| Command::Shutdown { respond_to: tx })
            .await??;
        info!("Uploader shutdown complete");
        Ok(())
    }
}

impl UploaderExt for Arc<Uploader> {
    async fn enqueue(&self, job: Job) -> Result<(), UploadErr> {
        self.as_ref().enqueue(job).await
    }

    async fn len(&self) -> Result<usize, UploadErr> {
        self.as_ref().len().await
    }

    async fn shutdown(&self) -> Result<(), UploadErr> {
        self.as_ref().shutdown().await
    }
}
