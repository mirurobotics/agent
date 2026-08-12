// standard crates
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::cooldown;
use crate::errors::HTTPCode;
use crate::trace;
use crate::upload::{
    errors::*,
    executor::UploadExecutor,
    job::Job,
    queue::{Queue, QueueEntry, QueueSnapshotFile},
};

// external crates
use chrono::{DateTime, TimeDelta, Utc};
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

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
    /// Maximum number of queued jobs (in-flight job excluded).
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
    /// Push a job at the tail of the queue.
    async fn enqueue(&self, job: Job) -> Result<(), UploadErr>;
    /// The number of queued jobs, excluding any in-flight job.
    async fn len(&self) -> Result<usize, UploadErr>;
    /// Stop the actor, dropping any in-flight upload future (see the cancel
    /// safety contract on [`UploadExecutor`]) and all queued jobs.
    async fn shutdown(&self) -> Result<(), UploadErr>;
}

// ================================== WORKER ======================================= //
pub(crate) enum Command {
    Enqueue {
        job: Job,
        respond_to: oneshot::Sender<Result<(), UploadErr>>,
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

pub(crate) struct Worker<ExecutorT, F, Fut, N>
where
    ExecutorT: UploadExecutor,
    F: Fn(Duration) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
    N: Fn() -> DateTime<Utc> + Send + Sync + 'static,
{
    receiver: Receiver<Command>,
    queue: Queue,
    executor: Arc<ExecutorT>,
    options: UploaderOptions,
    sleep_fn: F,
    now_fn: N,
}

impl<ExecutorT, F, Fut, N> Worker<ExecutorT, F, Fut, N>
where
    ExecutorT: UploadExecutor,
    F: Fn(Duration) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
    N: Fn() -> DateTime<Utc> + Send + Sync + 'static,
{
    pub(crate) async fn run(mut self) {
        loop {
            let now = (self.now_fn)();
            match self.queue.pop_ready(now).await {
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

    /// Drive one executor attempt on `entry`, while staying responsive to
    /// commands. On a retryable failure the entry is stamped with its
    /// next-attempt deadline and requeued at the tail — no sleeping here; at
    /// `options.attempts` total failures (or a terminal failure) it is
    /// dropped.
    async fn run_attempt(&mut self, mut entry: QueueEntry) -> Flow {
        entry.attempts += 1;

        let file = &entry.job.file;
        let rule = &entry.job.file_rule_id;
        let size = entry.job.size;
        let attempt = entry.attempts;
        info!("upload: attempting {file} rule {rule} size {size} attempt {attempt}");

        let err = match self.attempt_upload(&entry).await {
            AttemptOutcome::ShuttingDown => return Flow::Shutdown,
            AttemptOutcome::Succeeded => {
                Self::log_success(&entry);
                return Flow::Continue;
            }
            AttemptOutcome::Failed(err) => {
                let file = &entry.job.file;
                let attempt = entry.attempts;
                warn!("upload: attempt {attempt} for file {file} failed: {err:?}");
                err
            }
        };

        if let Some(status) = err.terminal_status() {
            Self::log_terminal_drop(&entry, status, &err);
            return Flow::Continue;
        }

        if entry.attempts >= self.options.attempts {
            Self::log_dropped(&entry, &err);
            return Flow::Continue;
        }

        let wait = cooldown::calc(&self.options.backoff, entry.attempts - 1).max(0);
        entry.next_attempt_at = Some((self.now_fn)() + TimeDelta::seconds(wait));
        let file = &entry.job.file;
        info!("upload: retrying {file} in {wait}s");
        self.requeue(entry).await;
        Flow::Continue
    }

    /// Wait out the shortest backoff among queued entries, staying responsive
    /// to commands. Deliberately NOT [`Self::run_until_shutdown`]: that helper
    /// keeps driving its future after handling a command, but an enqueue here
    /// must return to the run loop immediately so a newly eligible entry is
    /// re-evaluated rather than waiting out the sleep. Any non-shutdown
    /// command — like the sleep completing — returns [`Flow::Continue`].
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

    /// Requeue `entry` at the tail with its attempt count preserved.
    async fn requeue(&mut self, entry: QueueEntry) {
        let file = &entry.job.file;
        let attempts = entry.attempts;
        info!("upload: requeuing {file} at tail after {attempts} attempt(s)");
        self.queue.requeue(entry).await;
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

    fn log_terminal_drop(entry: &QueueEntry, status: HTTPCode, err: &UploadErr) {
        error!(
            "dropping upload job: backend rejected it with terminal HTTP status {status} \
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
                dispatch!(
                    self.queue.enqueue(job).await,
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
    pub fn spawn<ExecutorT, F, Fut, N>(
        buffer_size: usize,
        executor: Arc<ExecutorT>,
        options: UploaderOptions,
        snapshot_file: Option<QueueSnapshotFile>,
        sleep_fn: F,
        now_fn: N,
    ) -> Result<(Self, JoinHandle<()>), UploadErr>
    where
        ExecutorT: UploadExecutor + 'static,
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
        .await?
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
