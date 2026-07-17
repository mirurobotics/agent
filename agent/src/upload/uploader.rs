// standard crates
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::cooldown;
use crate::trace;
use crate::upload::{
    errors::{ReceiveActorMessageErr, SendActorMessageErr, UploadErr},
    executor::UploadExecutor,
    job::Job,
    queue::{Queue, QueueEntry, QueueSnapshotFile},
};

// external crates
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tracing::{error, info};

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
    /// Consecutive executor attempts on the same job (with backoff sleeps
    /// between them) before it is requeued at the tail.
    pub in_place_attempts: u32,
    /// Global per-job attempt cap across rounds; when exhausted, the job is
    /// dropped with a warning.
    pub max_total_attempts: u32,
    /// Backoff between in-place attempts; the exponent is the current round's
    /// attempt count minus one.
    pub backoff: cooldown::Backoff,
}

impl Default for UploaderOptions {
    fn default() -> Self {
        Self {
            queue_capacity: 1024,
            in_place_attempts: 3,
            max_total_attempts: 9,
            backoff: cooldown::Backoff {
                base_secs: 10,
                growth_factor: 2,
                max_secs: 120,
            },
        }
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
    /// The upload succeeded; the round is done.
    Succeeded,
    /// The upload failed; carries the error for logging and retry decisions.
    Failed(UploadErr),
    /// A shutdown arrived mid-attempt; the run loop must stop.
    ShuttingDown,
}

pub(crate) struct Worker<ExecutorT, F, Fut>
where
    ExecutorT: UploadExecutor,
    F: Fn(Duration) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    receiver: Receiver<Command>,
    queue: Queue,
    executor: Arc<ExecutorT>,
    options: UploaderOptions,
    sleep_fn: F,
}

impl<ExecutorT, F, Fut> Worker<ExecutorT, F, Fut>
where
    ExecutorT: UploadExecutor,
    F: Fn(Duration) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    pub(crate) async fn run(mut self) {
        loop {
            match self.queue.pop_front().await {
                // idle: nothing to interleave, just wait for the next command
                None => match self.receiver.recv().await {
                    // all senders dropped
                    None => break,
                    Some(cmd) => {
                        if let Flow::Shutdown = self.handle_command(cmd).await {
                            break;
                        }
                    }
                },
                Some(entry) => {
                    if let Flow::Shutdown = self.run_round(entry).await {
                        break;
                    }
                }
            }
        }
    }

    /// Drive up to `options.in_place_attempts` executor attempts on `entry`
    /// with backoff sleeps in between, while staying responsive to commands.
    /// A permanent (non-retryable) failure drops the job immediately. On a
    /// round-ending failure the job is requeued at the tail (no sleep); at
    /// `options.max_total_attempts` total failures it is dropped.
    async fn run_round(&mut self, mut entry: QueueEntry) -> Flow {
        for attempt_this_round in 1..=self.options.in_place_attempts {
            entry.attempts += 1;

            let err = match self.attempt_upload(&entry).await {
                AttemptOutcome::ShuttingDown => return Flow::Shutdown,
                AttemptOutcome::Succeeded => {
                    Self::log_success(&entry);
                    return Flow::Continue;
                }
                AttemptOutcome::Failed(err) => err,
            };

            if err.is_permanent() {
                Self::log_dropped_permanent(&entry, &err);
                return Flow::Continue;
            }

            if entry.attempts >= self.options.max_total_attempts {
                Self::log_dropped(&entry, &err);
                return Flow::Continue;
            }

            // round over: requeue at the tail with attempts preserved; no sleep on the
            // round-ending failure
            if attempt_this_round == self.options.in_place_attempts {
                self.requeue(entry).await;
                return Flow::Continue;
            }

            // back off before the next in-place attempt
            if let Flow::Shutdown = self.await_next_round(attempt_this_round).await {
                return Flow::Shutdown;
            }
        }
        Flow::Continue
    }

    /// Drive one executor attempt on `entry` to completion while staying
    /// responsive to commands. The future is built from clones so it borrows
    /// nothing from `self`, leaving `self` free to serve commands while it runs.
    async fn attempt_upload(&mut self, entry: &QueueEntry) -> AttemptOutcome {
        let executor = self.executor.clone();
        let job = entry.job.clone();
        match self
            .run_until_shutdown(async move { executor.upload(&job).await })
            .await
        {
            None => AttemptOutcome::ShuttingDown,
            Some(Ok(())) => AttemptOutcome::Succeeded,
            Some(Err(err)) => AttemptOutcome::Failed(err),
        }
    }

    /// Requeue `entry` at the tail with its attempt count preserved, dropping
    /// it with a warning if the queue rejects it.
    async fn requeue(&mut self, entry: QueueEntry) {
        let job = entry.job.clone();
        if let Err(requeue_err) = self.queue.requeue(entry).await {
            error!(
                "dropping upload job (rule {}, file {}, digest {}): requeue failed: {requeue_err:?}",
                job.upload_rule_id, job.file, job.digest
            );
        }
    }

    /// Back off before the next in-place attempt, staying responsive to commands.
    /// `attempt_this_round` is the number of attempts made in the current round; the
    /// backoff exponent is that minus one. Returns [`Flow::Shutdown`] if a shutdown
    /// arrived during the sleep.
    async fn await_next_round(&mut self, attempt_this_round: u32) -> Flow {
        let secs = cooldown::calc(&self.options.backoff, attempt_this_round - 1);
        let sleep_fut = (self.sleep_fn)(Duration::from_secs(secs.max(0) as u64));
        match self.run_until_shutdown(sleep_fut).await {
            None => Flow::Shutdown,
            Some(()) => Flow::Continue,
        }
    }

    fn log_success(entry: &QueueEntry) {
        info!(
            "uploaded file {} (rule {}, digest {}) on attempt {}",
            entry.job.file, entry.job.upload_rule_id, entry.job.digest, entry.attempts
        );
    }

    fn log_dropped(entry: &QueueEntry, err: &UploadErr) {
        error!(
            "dropping upload job after {} attempts (rule {}, file {}, digest {}): {err:?}",
            entry.attempts, entry.job.upload_rule_id, entry.job.file, entry.job.digest
        );
    }

    fn log_dropped_permanent(entry: &QueueEntry, err: &UploadErr) {
        error!(
            "dropping upload job after {} attempt(s) (rule {}, file {}, digest {}): permanent client error, not retrying: {err:?}",
            entry.attempts, entry.job.upload_rule_id, entry.job.file, entry.job.digest
        );
    }

    /// Run `fut` to completion while continuing to serve commands. Returns
    /// `None` when a shutdown arrived (or all senders dropped), in which case
    /// `fut` is dropped — cancelling an in-flight upload or sleep.
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
    pub fn spawn<ExecutorT, F, Fut>(
        buffer_size: usize,
        executor: Arc<ExecutorT>,
        options: UploaderOptions,
        snapshot_file: Option<QueueSnapshotFile>,
        sleep_fn: F,
    ) -> Result<(Self, JoinHandle<()>), UploadErr>
    where
        ExecutorT: UploadExecutor + 'static,
        F: Fn(Duration) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
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
