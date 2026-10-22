// internal crates
use crate::data_uploads::retention::{
    deleter_impl::{DeleterArgs, DeleterImpl},
    errors::*,
    job::Job,
};
use crate::trace;

// external crates
use tokio::sync::{mpsc, oneshot};
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
// `-> impl Future + Send` (not `async fn`) so callers awaiting a generic
// `D: DeleterExt` inside their own `Send` futures can prove those futures
// `Send`.
//
// an async, actor-round-tripping is_empty would be dead weight next to len()
#[allow(clippy::len_without_is_empty)]
pub trait DeleterExt: Send + Sync {
    fn enqueue(&self, job: Job) -> impl std::future::Future<Output = Result<(), DeleteErr>> + Send;
    /// Run one deletion pass over the queued jobs.
    fn sweep(&self) -> impl std::future::Future<Output = Result<(), DeleteErr>> + Send;
    /// The number of queued jobs, including one currently being swept: a
    /// selected job stays queued until its sweep resolves it.
    fn len(&self) -> impl std::future::Future<Output = Result<usize, DeleteErr>> + Send;
    /// Stop the actor. Queued jobs stay in the persisted snapshot and are
    /// re-seeded on the next spawn.
    fn shutdown(&self) -> impl std::future::Future<Output = Result<(), DeleteErr>> + Send;
}

// ================================== WORKER ======================================= //
pub(crate) enum Command {
    Enqueue {
        job: Job,
        respond_to: oneshot::Sender<Result<(), DeleteErr>>,
    },
    Sweep {
        respond_to: oneshot::Sender<Result<(), DeleteErr>>,
    },
    Len {
        respond_to: oneshot::Sender<Result<usize, DeleteErr>>,
    },
    Shutdown {
        respond_to: oneshot::Sender<Result<(), DeleteErr>>,
    },
}

pub(crate) struct Worker {
    deleter: DeleterImpl,
    receiver: mpsc::Receiver<Command>,
}

impl Worker {
    pub(crate) async fn run(mut self) {
        while let Some(cmd) = self.receiver.recv().await {
            match cmd {
                Command::Shutdown { respond_to } => {
                    self.deleter.save_uptime().await;
                    if let Err(e) = respond_to.send(Ok(())) {
                        error!("Actor failed to send shutdown response: {:?}", e);
                    }
                    break;
                }
                Command::Enqueue { job, respond_to } => {
                    dispatch!(
                        self.deleter.enqueue(job).await,
                        respond_to,
                        "Actor failed to send enqueue response"
                    );
                }
                Command::Sweep { respond_to } => {
                    dispatch!(
                        self.deleter.sweep().await,
                        respond_to,
                        "Actor failed to send sweep response"
                    );
                }
                Command::Len { respond_to } => {
                    dispatch!(
                        Ok(self.deleter.len()),
                        respond_to,
                        "Actor failed to send len response"
                    );
                }
            }
        }
    }
}

// ================================== HANDLE ======================================= //
/// Command handle to the [`DeleterImpl`] actor. Reactive, not
/// self-scheduling: each [`sweep`](DeleterExt::sweep) call walks the queue
/// one job at a time. The cadence that drives repeated sweeps is imposed
/// by an external driver, not by this type. A sweep may safely be
/// interleaved with commands: a selected job never leaves the queue until
/// it resolves, so a persist triggered by another command cannot write the
/// in-flight job out of the snapshot.
#[derive(Debug)]
pub struct Deleter {
    sender: mpsc::Sender<Command>,
}

impl Deleter {
    pub fn spawn(
        buffer_size: usize,
        args: DeleterArgs,
    ) -> Result<(Self, JoinHandle<()>), DeleteErr> {
        let (sender, receiver) = mpsc::channel(buffer_size);
        let worker = Worker {
            deleter: DeleterImpl::new(args),
            receiver,
        };
        let worker_handle = tokio::spawn(worker.run());
        Ok((Self { sender }, worker_handle))
    }

    async fn send_command<R>(
        &self,
        cmd: impl FnOnce(oneshot::Sender<R>) -> Command,
    ) -> Result<R, DeleteErr> {
        let (send, recv) = oneshot::channel();
        self.sender.send(cmd(send)).await.map_err(|e| {
            DeleteErr::SendActorMessageErr(SendActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })?;
        recv.await.map_err(|e| {
            DeleteErr::ReceiveActorMessageErr(ReceiveActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })
    }
}

impl DeleterExt for Deleter {
    async fn enqueue(&self, job: Job) -> Result<(), DeleteErr> {
        self.send_command(|tx| Command::Enqueue {
            job,
            respond_to: tx,
        })
        .await?
    }

    async fn sweep(&self) -> Result<(), DeleteErr> {
        self.send_command(|tx| Command::Sweep { respond_to: tx })
            .await?
    }

    async fn len(&self) -> Result<usize, DeleteErr> {
        self.send_command(|tx| Command::Len { respond_to: tx })
            .await?
    }

    async fn shutdown(&self) -> Result<(), DeleteErr> {
        self.send_command(|tx| Command::Shutdown { respond_to: tx })
            .await??;
        info!("delete: deleter shutdown complete");
        Ok(())
    }
}
