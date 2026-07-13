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
    job::{DedupKey, Job},
    queue::{Outcome, PendingJob, Queue},
};

// external crates
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
    /// Consecutive executor attempts on the same job (with backoff sleeps
    /// between them) before it is requeued at the tail.
    pub in_place_attempts: u32,
    /// Global per-job attempt cap across rounds; when exhausted, the job is
    /// dropped with a warning.
    pub max_total_attempts: u32,
    /// Backoff between in-place attempts; the exponent is the job's total
    /// attempt count so far minus one.
    pub backoff: cooldown::Backoff,
}

impl Default for UploaderOptions {
    fn default() -> Self {
        Self {
            queue_capacity: 1024,
            in_place_attempts: 3,
            max_total_attempts: 9,
            backoff: cooldown::Backoff {
                base_secs: 1,
                growth_factor: 2,
                max_secs: 30,
            },
        }
    }
}

// =================================== TRAIT ======================================= //
#[allow(async_fn_in_trait)]
// an async, actor-round-tripping is_empty would be dead weight next to len()
#[allow(clippy::len_without_is_empty)]
pub trait UploaderExt: Send + Sync {
    /// Push a job at the tail of the queue. Returns `Ok(Duplicate)` without
    /// enqueuing when a key-equal job is already queued or in flight.
    async fn enqueue(&self, job: Job) -> Result<Outcome, UploadErr>;
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
        respond_to: oneshot::Sender<Result<Outcome, UploadErr>>,
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
    /// The dedup key of the job currently being processed, claimed for the
    /// whole round (uploads and backoff sleeps) so duplicate enqueues of the
    /// in-flight job are rejected with no window for one to slip in.
    in_flight: Option<DedupKey>,
}

impl<ExecutorT, F, Fut> Worker<ExecutorT, F, Fut>
where
    ExecutorT: UploadExecutor,
    F: Fn(Duration) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    pub(crate) async fn run(mut self) {
        loop {
            match self.queue.pop_front() {
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
                Some(pending) => {
                    if let Flow::Shutdown = self.process(pending).await {
                        break;
                    }
                }
            }
        }
    }

    /// Run one round of in-place attempts on `pending`, claiming its dedup
    /// key as in-flight for the whole round.
    async fn process(&mut self, pending: PendingJob) -> Flow {
        self.in_flight = Some(pending.job.dedup_key());
        let flow = self.run_round(pending).await;
        self.in_flight = None;
        flow
    }

    /// Drive up to `options.in_place_attempts` executor attempts on `pending`
    /// with backoff sleeps in between, while staying responsive to commands.
    /// On a round-ending failure the job is requeued at the tail (no sleep);
    /// at `options.max_total_attempts` total failures it is dropped.
    async fn run_round(&mut self, mut pending: PendingJob) -> Flow {
        for attempt_in_round in 1..=self.options.in_place_attempts {
            pending.attempts += 1;

            // the future is built from clones so it borrows nothing from
            // self, leaving self free to serve commands while it runs
            let executor = self.executor.clone();
            let job = pending.job.clone();
            let result = match self
                .drive_interleaved(async move { executor.upload(&job).await })
                .await
            {
                None => return Flow::Shutdown,
                Some(result) => result,
            };

            let err = match result {
                Ok(()) => {
                    info!(
                        "uploaded file {} (rule {}, digest {}) on attempt {}",
                        pending.job.file,
                        pending.job.upload_rule_id,
                        pending.job.digest,
                        pending.attempts
                    );
                    return Flow::Continue;
                }
                Err(err) => err,
            };

            if pending.attempts >= self.options.max_total_attempts {
                warn!(
                    "dropping upload job after {} attempts (rule {}, file {}, digest {}): {err:?}",
                    pending.attempts,
                    pending.job.upload_rule_id,
                    pending.job.file,
                    pending.job.digest
                );
                return Flow::Continue;
            }

            // round over: requeue at the tail with attempts preserved; no
            // sleep on the round-ending failure
            if attempt_in_round == self.options.in_place_attempts {
                let job = pending.job.clone();
                if let Err(requeue_err) = self.queue.requeue(pending).await {
                    warn!(
                        "dropping upload job (rule {}, file {}, digest {}): requeue failed: {requeue_err:?}",
                        job.upload_rule_id, job.file, job.digest
                    );
                }
                return Flow::Continue;
            }

            // back off before the next in-place attempt
            let secs = cooldown::calc(&self.options.backoff, pending.attempts - 1);
            let sleep_fut = (self.sleep_fn)(Duration::from_secs(secs.max(0) as u64));
            if self.drive_interleaved(sleep_fut).await.is_none() {
                return Flow::Shutdown;
            }
        }
        Flow::Continue
    }

    /// Drive `fut` to completion while continuing to serve commands. Returns
    /// `None` when a shutdown arrived (or all senders dropped), in which case
    /// `fut` is dropped — cancelling an in-flight upload or sleep.
    async fn drive_interleaved<T>(&mut self, fut: impl Future<Output = T>) -> Option<T> {
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
                let result = if self.in_flight.as_ref() == Some(&job.dedup_key()) {
                    info!(
                        "upload job for file {} (rule {}) is in flight; skipping enqueue",
                        job.file, job.upload_rule_id
                    );
                    Ok(Outcome::Duplicate)
                } else {
                    self.queue.enqueue(job).await
                };
                dispatch!(result, respond_to, "Actor failed to send enqueue response");
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
        sleep_fn: F,
    ) -> Result<(Self, JoinHandle<()>), UploadErr>
    where
        ExecutorT: UploadExecutor + 'static,
        F: Fn(Duration) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let (sender, receiver) = mpsc::channel(buffer_size);
        let worker = Worker {
            receiver,
            queue: Queue::new(options.queue_capacity),
            executor,
            options,
            sleep_fn,
            in_flight: None,
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
    async fn enqueue(&self, job: Job) -> Result<Outcome, UploadErr> {
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
    async fn enqueue(&self, job: Job) -> Result<Outcome, UploadErr> {
        self.as_ref().enqueue(job).await
    }

    async fn len(&self) -> Result<usize, UploadErr> {
        self.as_ref().len().await
    }

    async fn shutdown(&self) -> Result<(), UploadErr> {
        self.as_ref().shutdown().await
    }
}
