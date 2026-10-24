// standard crates
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::data_uploads::{
    retention::DeleterExt,
    upload::{
        checkpoint::{ChannelCheckpointer, CheckpointMsg},
        errors::*,
        executor::UploadExecutor,
        job::Job,
        queue::{QueueEntry, QueueSnapshotFile},
        uploader_impl::{UploaderImpl, UploaderOptions},
    },
};
use crate::queue::NextAttempt;
use crate::trace;

// external crates
use chrono::{DateTime, Utc};
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

// =================================== TRAIT ======================================= //
#[allow(async_fn_in_trait)]
// an async, actor-round-tripping is_empty would be dead weight next to len()
#[allow(clippy::len_without_is_empty)]
pub trait UploaderExt: Send + Sync {
    /// Push a job at the tail of the queue.
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
        job: Box<Job>,
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

pub(crate) struct Worker<ExecutorT, D, F, Fut, N>
where
    ExecutorT: UploadExecutor,
    D: DeleterExt + 'static,
    F: Fn(Duration) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
    N: Fn() -> DateTime<Utc> + Send + Sync + 'static,
{
    receiver: Receiver<Command>,
    uploader: UploaderImpl<ExecutorT, D, N>,
    sleep_fn: F,
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
            match self.uploader.next_attempt().await {
                NextAttempt::Ready(entry) => {
                    if let Flow::Shutdown = self.run_attempt(entry).await {
                        break;
                    }
                }
                // idle: nothing to interleave, just wait for the next command
                NextAttempt::Empty => {
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
                NextAttempt::Backoff(wait) => {
                    if let Flow::Shutdown = self.idle_wait(wait).await {
                        break;
                    }
                }
            }
        }
    }

    /// Drive one executor attempt on `entry` to completion while staying
    /// responsive to commands, then resolve the entry with its result.
    async fn run_attempt(&mut self, entry: QueueEntry) -> Flow {
        let (checkpoint_tx, mut checkpoint_rx) = mpsc::channel::<CheckpointMsg>(1);
        let attempt = self
            .uploader
            .attempt(&entry, ChannelCheckpointer::new(checkpoint_tx));
        tokio::pin!(attempt);
        loop {
            tokio::select! {
                result = &mut attempt => {
                    self.uploader.finish_attempt(entry, result).await;
                    return Flow::Continue;
                }
                // disabled once the attempt drops its checkpointer
                Some((checkpoint, ack)) = checkpoint_rx.recv() => {
                    self.uploader.save_checkpoint(&entry, checkpoint).await;
                    let _ = ack.send(());
                }
                // recv is cancel-safe: losing the race never drops a command.
                // A shutdown drops the attempt and leaves the entry queued,
                // with its checkpoints, so the next boot picks it up.
                cmd = self.receiver.recv() => match cmd {
                    None => return Flow::Shutdown,
                    Some(cmd) => {
                        if let Flow::Shutdown = self.handle_command(cmd).await {
                            return Flow::Shutdown;
                        }
                    }
                }
            }
        }
    }

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
                    self.uploader.enqueue(*job).await,
                    respond_to,
                    "Actor failed to send enqueue response"
                );
                Flow::Continue
            }
            Command::Len { respond_to } => {
                dispatch!(
                    Ok(self.uploader.len()),
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
        let worker = Worker {
            receiver,
            uploader: UploaderImpl::new(executor, deleter, options, snapshot_file, now_fn),
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
            job: Box::new(job),
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
