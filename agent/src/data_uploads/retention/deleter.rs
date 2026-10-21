// internal crates
use crate::actor::{self, Actor, Flow, Mailbox};
use crate::data_uploads::retention::{
    deleter_impl::{DeleterArgs, DeleterImpl},
    errors::*,
    job::Job,
};

// external crates
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::info;

const LABEL: &str = "delete";

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
    commands: mpsc::Receiver<Command>,
}

impl Worker {
    pub(crate) async fn run(mut self) {
        while let Flow::Continue = actor::serve_next(&mut self).await {}
    }
}

impl Actor for Worker {
    type Command = Command;

    fn commands(&mut self) -> &mut mpsc::Receiver<Command> {
        &mut self.commands
    }

    async fn handle(&mut self, cmd: Command) -> Flow {
        match cmd {
            Command::Shutdown { respond_to } => {
                actor::respond(LABEL, "shutdown", respond_to, Ok(()));
                return Flow::Shutdown;
            }
            Command::Enqueue { job, respond_to } => {
                let result = self.deleter.enqueue(job).await;
                actor::respond(LABEL, "enqueue", respond_to, result);
            }
            Command::Sweep { respond_to } => {
                let result = self.deleter.sweep().await;
                actor::respond(LABEL, "sweep", respond_to, result);
            }
            Command::Len { respond_to } => {
                actor::respond(LABEL, "len", respond_to, Ok(self.deleter.len()));
            }
        }
        Flow::Continue
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
    mailbox: Mailbox<Command>,
}

impl Deleter {
    pub fn spawn(
        buffer_size: usize,
        args: DeleterArgs,
    ) -> Result<(Self, JoinHandle<()>), DeleteErr> {
        let (mailbox, commands) = actor::channel(LABEL, buffer_size);
        let worker = Worker {
            deleter: DeleterImpl::new(args),
            commands,
        };
        let worker_handle = tokio::spawn(worker.run());
        Ok((Self { mailbox }, worker_handle))
    }

    async fn send_command<R>(
        &self,
        op: &str,
        make_cmd: impl FnOnce(oneshot::Sender<R>) -> Command,
    ) -> Result<R, DeleteErr> {
        Ok(self.mailbox.request(op, make_cmd).await?)
    }
}

impl DeleterExt for Deleter {
    async fn enqueue(&self, job: Job) -> Result<(), DeleteErr> {
        self.send_command("enqueue", |tx| Command::Enqueue {
            job,
            respond_to: tx,
        })
        .await?
    }

    async fn sweep(&self) -> Result<(), DeleteErr> {
        self.send_command("sweep", |tx| Command::Sweep { respond_to: tx })
            .await?
    }

    async fn len(&self) -> Result<usize, DeleteErr> {
        self.send_command("len", |tx| Command::Len { respond_to: tx })
            .await?
    }

    async fn shutdown(&self) -> Result<(), DeleteErr> {
        self.send_command("shutdown", |tx| Command::Shutdown { respond_to: tx })
            .await??;
        info!("delete: deleter shutdown complete");
        Ok(())
    }
}
