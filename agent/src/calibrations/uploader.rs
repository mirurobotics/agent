// standard crates
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::calibrations::{
    errors::{CalibrationsErr, ReceiveActorMessageErr, SendActorMessageErr},
    report::{QueueEntry, QueueSnapshotFile, Report},
    sender::Sender,
    uploader_impl::{NextAttempt, UploaderImpl, UploaderOptions},
};
use crate::filesys::Dir;
use crate::sync::syncer::{next_sync_event, SyncEvent};
use crate::trace;

// external crates
use chrono::{DateTime, Utc};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tracing::{error, info};
use uuid::Uuid;

pub struct UploaderArgs<S, F, N> {
    /// Where the queue persists; `None` keeps it in memory only.
    pub snapshot_file: Option<QueueSnapshotFile>,
    /// The directory holding one data file per queued report.
    pub reports: Dir,
    pub sender: Arc<S>,
    /// Sync events; a successful sync makes every waiting report due at once. `None` disables
    /// the early retry.
    pub events: Option<watch::Receiver<SyncEvent>>,
    pub options: UploaderOptions,
    pub sleep_fn: F,
    /// The clock that report backoffs and ages are measured against.
    pub now_fn: N,
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

struct Worker<S, F, N> {
    receiver: mpsc::Receiver<Command>,
    uploader: UploaderImpl<S, N>,
    events: Option<watch::Receiver<SyncEvent>>,
    sleep_fn: F,
}

impl<S, F, Fut, N> Worker<S, F, N>
where
    S: Sender + 'static,
    F: Fn(Duration) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
    N: Fn() -> DateTime<Utc> + Send + Sync + 'static,
{
    async fn run(mut self) {
        self.uploader.sweep().await;
        loop {
            let flow = match self.uploader.next_attempt().await {
                NextAttempt::Ready(entry) => self.attempt(entry).await,
                NextAttempt::Empty => self.idle().await,
                NextAttempt::Backoff(wait) => self.back_off(wait).await,
            };
            if let Flow::Shutdown = flow {
                break;
            }
        }
        info!("calibrations: uploader shutdown complete");
    }

    /// Upload the report queued as `entry` and settle it. A shutdown
    /// mid-upload leaves the report queued, to be re-sent on the next boot.
    async fn attempt(&mut self, entry: QueueEntry) -> Flow {
        let upload = self.uploader.upload(&entry);
        let Some(result) = self.serve_until(upload).await else {
            return Flow::Shutdown;
        };
        self.uploader.finish(entry, result).await;
        Flow::Continue
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

    /// Wait `wait` for the earliest report to come due, while serving
    /// commands. A new report or a successful sync (Miru is reachable again,
    /// so every waiting report is made due) ends the wait early.
    async fn back_off(&mut self, wait: Duration) -> Flow {
        let sleep = (self.sleep_fn)(wait);
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                _ = &mut sleep => return Flow::Continue,
                cmd = self.receiver.recv() => {
                    let is_new_report = matches!(cmd, Some(Command::Enqueue { .. }));
                    let flow = self.handle(cmd).await;
                    if is_new_report || matches!(flow, Flow::Shutdown) {
                        return flow;
                    }
                }
                event = next_sync_event(&mut self.events) => {
                    if self.reconnected(event) {
                        self.uploader.retry_now().await;
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
/// time, oldest due first, retrying a failed report at the tail after its own
/// backoff. It keeps serving intake during an upload or a backoff.
#[derive(Debug)]
pub struct Uploader {
    sender: mpsc::Sender<Command>,
}

impl Uploader {
    pub fn spawn<S, F, Fut, N>(
        buffer_size: usize,
        args: UploaderArgs<S, F, N>,
    ) -> (Self, JoinHandle<()>)
    where
        S: Sender + 'static,
        F: Fn(Duration) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
        N: Fn() -> DateTime<Utc> + Send + Sync + 'static,
    {
        let (sender, receiver) = mpsc::channel(buffer_size);
        let worker = Worker {
            receiver,
            uploader: UploaderImpl::new(
                args.sender,
                args.snapshot_file,
                args.reports,
                args.options,
                args.now_fn,
            ),
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
