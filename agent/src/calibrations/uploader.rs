// standard crates
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::calibrations::{
    errors::{CalibrationsErr, ReceiveActorMessageErr, SendActorMessageErr},
    queue::{DirQueue, Report},
    sender::Sender,
    uploader_impl::{UploaderImpl, UploaderOptions},
};
use crate::sync::syncer::SyncEvent;
use crate::trace;
use crate::workers::next_sync_event;

// external crates
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tracing::{error, info};
use uuid::Uuid;

pub struct UploaderArgs<S, F> {
    pub queue: DirQueue,
    pub sender: Arc<S>,
    /// Sync events; a successful sync ends a backoff early. `None` disables
    /// the early retry.
    pub events: Option<watch::Receiver<SyncEvent>>,
    pub options: UploaderOptions,
    pub sleep_fn: F,
}

// ================================== WORKER ======================================= //
enum Command {
    Enqueue {
        report: Box<Report>,
        respond_to: oneshot::Sender<Result<Uuid, CalibrationsErr>>,
    },
    Len {
        respond_to: oneshot::Sender<usize>,
    },
    Shutdown {
        respond_to: oneshot::Sender<()>,
    },
}

enum Flow {
    Continue,
    Shutdown,
}

struct Worker<S, F> {
    receiver: mpsc::Receiver<Command>,
    uploader: UploaderImpl,
    sender: Arc<S>,
    events: Option<watch::Receiver<SyncEvent>>,
    sleep_fn: F,
}

impl<S, F, Fut> Worker<S, F>
where
    S: Sender + 'static,
    F: Fn(Duration) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    async fn run(mut self) {
        loop {
            let flow = match self.uploader.next_ready() {
                Some(id) => self.attempt(id).await,
                None => self.idle().await,
            };
            if let Flow::Shutdown = flow {
                break;
            }
        }
        info!("calibrations: uploader shutdown complete");
    }

    /// Send report `id`, settle it, then wait out any backoff. A shutdown
    /// mid-send leaves the report queued, to be re-sent on the next boot.
    async fn attempt(&mut self, id: Uuid) -> Flow {
        let result = match self.uploader.read(id).await {
            Ok(Some(report)) => {
                let sender = self.sender.clone();
                let send = self.serve_until(async move { sender.send(&report).await });
                let Some(result) = send.await else {
                    return Flow::Shutdown;
                };
                result
            }
            Ok(None) => return Flow::Continue,
            Err(err) => Err(err),
        };
        match self.uploader.finish(id, result).await {
            Some(wait) => self.back_off(wait).await,
            None => Flow::Continue,
        }
    }

    /// Drive `fut` to completion while serving commands, so intake never
    /// waits on Miru. `None` when a shutdown arrived; `fut` is then dropped.
    async fn serve_until<T>(&mut self, fut: impl Future<Output = T>) -> Option<T> {
        tokio::pin!(fut);
        loop {
            tokio::select! {
                result = &mut fut => return Some(result),
                // recv is cancel-safe: losing the race never drops a command
                cmd = self.receiver.recv() => {
                    if let Flow::Shutdown = self.handle(cmd).await {
                        return None;
                    }
                }
            }
        }
    }

    /// Wait out `wait` while serving commands, ending early when a sync
    /// succeeds (Miru is reachable again). The sleep is never restarted, so
    /// intake can't extend a backoff.
    async fn back_off(&mut self, wait: Duration) -> Flow {
        let sleep = (self.sleep_fn)(wait);
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                _ = &mut sleep => return Flow::Continue,
                cmd = self.receiver.recv() => {
                    if let Flow::Shutdown = self.handle(cmd).await {
                        return Flow::Shutdown;
                    }
                }
                event = next_sync_event(&mut self.events) => {
                    if self.reconnected(event) {
                        return Flow::Continue;
                    }
                }
            }
        }
    }

    /// Wait for a command or a sync event with nothing ready to send.
    async fn idle(&mut self) -> Flow {
        tokio::select! {
            cmd = self.receiver.recv() => self.handle(cmd).await,
            event = next_sync_event(&mut self.events) => {
                self.reconnected(event);
                Flow::Continue
            }
        }
    }

    /// Whether `event` is a successful sync. An ended event stream disables the
    /// subscription.
    fn reconnected(&mut self, event: Option<SyncEvent>) -> bool {
        match event {
            Some(SyncEvent::SyncSuccess) => true,
            Some(_) => false,
            None => {
                error!("calibrations: syncer event stream ended; no early retry on reconnect");
                self.events = None;
                false
            }
        }
    }

    /// Serve `cmd`. Every handle dropping counts as a shutdown.
    async fn handle(&mut self, cmd: Option<Command>) -> Flow {
        let Some(cmd) = cmd else {
            return Flow::Shutdown;
        };
        match cmd {
            Command::Enqueue { report, respond_to } => {
                let result = self.uploader.enqueue(*report).await;
                respond(respond_to, result, "enqueue");
            }
            Command::Len { respond_to } => respond(respond_to, self.uploader.len(), "len"),
            Command::Shutdown { respond_to } => {
                respond(respond_to, (), "shutdown");
                return Flow::Shutdown;
            }
        }
        Flow::Continue
    }
}

fn respond<T>(respond_to: oneshot::Sender<T>, value: T, op: &str) {
    if respond_to.send(value).is_err() {
        error!("calibrations: failed to send the {op} response");
    }
}

// ================================== HANDLE ======================================= //
/// Handle to the calibration uploader: one task that owns the calibration
/// queue, queues what intake sends it, and uploads reports to Miru one at a
/// time, oldest first. It keeps serving intake during an upload or a backoff.
#[derive(Debug)]
pub struct Uploader {
    sender: mpsc::Sender<Command>,
}

impl Uploader {
    pub fn spawn<S, F, Fut>(buffer_size: usize, args: UploaderArgs<S, F>) -> (Self, JoinHandle<()>)
    where
        S: Sender + 'static,
        F: Fn(Duration) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let (sender, receiver) = mpsc::channel(buffer_size);
        let worker = Worker {
            receiver,
            uploader: UploaderImpl::new(args.queue, args.options),
            sender: args.sender,
            events: args.events,
            sleep_fn: args.sleep_fn,
        };
        (Self { sender }, tokio::spawn(worker.run()))
    }

    /// Queue `report` durably. A full queue is the 507
    /// [`CalibrationsErr::QueueFullErr`], and a report over the size cap the
    /// 413 [`CalibrationsErr::TooLargeErr`].
    pub async fn enqueue(&self, report: Report) -> Result<Uuid, CalibrationsErr> {
        self.send_command("enqueue", |respond_to| Command::Enqueue {
            report: Box::new(report),
            respond_to,
        })
        .await?
    }

    /// The number of queued reports, including one being uploaded.
    // an actor-round-tripping is_empty would be dead weight next to len()
    #[allow(clippy::len_without_is_empty)]
    pub async fn len(&self) -> Result<usize, CalibrationsErr> {
        self.send_command("len", |respond_to| Command::Len { respond_to })
            .await
    }

    /// Stop the uploader, dropping any in-flight upload. Queued reports,
    /// including the interrupted one, stay on disk for the next boot.
    pub async fn shutdown(&self) -> Result<(), CalibrationsErr> {
        info!("calibrations: shutting down uploader...");
        self.send_command("shutdown", |respond_to| Command::Shutdown { respond_to })
            .await
    }

    async fn send_command<R>(
        &self,
        op: &str,
        make_cmd: impl FnOnce(oneshot::Sender<R>) -> Command,
    ) -> Result<R, CalibrationsErr> {
        let (send, recv) = oneshot::channel();
        self.sender.send(make_cmd(send)).await.map_err(|e| {
            error!("calibrations: failed to send the {op} command to the uploader: {e}");
            CalibrationsErr::SendActorMessageErr(SendActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })?;
        recv.await.map_err(|e| {
            error!("calibrations: failed to receive the {op} response from the uploader: {e}");
            CalibrationsErr::ReceiveActorMessageErr(ReceiveActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })
    }
}
