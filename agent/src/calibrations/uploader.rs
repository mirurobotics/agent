// standard crates
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::buffer::BufferErr;
use crate::calibrations::{
    buffer::{Buffer, Report},
    errors::{CalibrationsErr, InvalidReportErr, ReceiveActorMessageErr, SendActorMessageErr},
    sender::Sender,
};
use crate::cooldown;
use crate::errors::Error;
use crate::filesys::{errors::ParseJSONErr, Dir, FileSysErr};
use crate::sync::syncer::SyncEvent;
use crate::trace;
use crate::workers::next_sync_event;

// external crates
use chrono::Utc;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tracing::{error, info, warn};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct UploaderOptions {
    pub backoff: cooldown::Backoff,
}

impl Default for UploaderOptions {
    /// The data-upload backoff (`upload::UploaderOptions::default().backoff`):
    /// 10 s, doubling, capped at 1 h.
    fn default() -> Self {
        Self {
            backoff: cooldown::Backoff {
                base_secs: 10,
                growth_factor: 2,
                max_secs: 3600,
            },
        }
    }
}

pub struct UploaderArgs<S, F> {
    pub buffer: Buffer,
    /// Where reports Miru permanently rejected are parked.
    pub failed: Dir,
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

/// How one attempt at the oldest buffered report ended.
enum Outcome {
    /// Sent, parked or dropped: the report is no longer buffered.
    Resolved,
    /// The report stays at the head of the buffer and is retried after a wait.
    Retry { network: bool },
}

/// The run of consecutive non-network failures, in memory only, so a restart
/// retries at once.
struct Streak(u32);

impl Streak {
    /// How long to wait after `outcome`. Network errors wait a flat
    /// `base_secs` and leave the streak alone; other failures grow it.
    fn next_wait(&mut self, outcome: &Outcome, backoff: &cooldown::Backoff) -> Option<Duration> {
        let secs = match outcome {
            Outcome::Resolved => {
                self.0 = 0;
                return None;
            }
            Outcome::Retry { network: true } => backoff.base_secs,
            Outcome::Retry { network: false } => {
                self.0 = self.0.saturating_add(1);
                cooldown::calc(backoff, self.0 - 1)
            }
        };
        Some(Duration::from_secs(secs.max(0) as u64))
    }
}

struct Worker<S, F> {
    receiver: mpsc::Receiver<Command>,
    buffer: Buffer,
    failed: Dir,
    sender: Arc<S>,
    events: Option<watch::Receiver<SyncEvent>>,
    options: UploaderOptions,
    sleep_fn: F,
    streak: Streak,
}

impl<S, F, Fut> Worker<S, F>
where
    S: Sender + 'static,
    F: Fn(Duration) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    async fn run(mut self) {
        loop {
            let next = self.buffer.next_ready(Utc::now()).map(|entry| entry.id);
            let flow = match next {
                Some(id) => self.attempt(id).await,
                None => self.idle().await,
            };
            if let Flow::Shutdown = flow {
                break;
            }
        }
        info!("calibrations: uploader shutdown complete");
    }

    /// Try report `id`, then wait out any backoff.
    async fn attempt(&mut self, id: Uuid) -> Flow {
        let Some(outcome) = self.upload(id).await else {
            return Flow::Shutdown;
        };
        match self.streak.next_wait(&outcome, &self.options.backoff) {
            Some(wait) => self.back_off(wait).await,
            None => Flow::Continue,
        }
    }

    /// Send report `id` and resolve it. `None` when a shutdown arrived
    /// mid-send; the report stays buffered and is re-sent on the next boot.
    async fn upload(&mut self, id: Uuid) -> Option<Outcome> {
        let result = match self.buffer.read(id).await {
            Ok(report) => {
                let sender = self.sender.clone();
                self.serve_until(async move { sender.send(&report).await })
                    .await?
            }
            Err(BufferErr::FileSysErr(FileSysErr::PathDoesNotExistErr(_))) => {
                return Some(self.drop_missing(id).await);
            }
            Err(BufferErr::FileSysErr(FileSysErr::ParseJSONErr(e))) => Err(unreadable(e)),
            Err(e) => Err(e.into()),
        };
        Some(self.resolve(id, result).await)
    }

    /// A report whose file is gone has nothing left to send or park.
    async fn drop_missing(&mut self, id: Uuid) -> Outcome {
        error!("calibrations: dropping report {id}, whose file is missing");
        self.buffer.remove(id).await;
        Outcome::Resolved
    }

    /// Remove a sent report, park one that can never be sent, and keep any
    /// other at the head of the buffer for a retry.
    async fn resolve(&mut self, id: Uuid, result: Result<(), CalibrationsErr>) -> Outcome {
        let err = match result {
            Ok(()) => {
                self.buffer.remove(id).await;
                info!("calibrations: uploaded report {id}");
                return Outcome::Resolved;
            }
            Err(err) => err,
        };
        if !err.is_terminal() {
            warn!("calibrations: failed to upload report {id}; will retry: {err}");
            return Outcome::Retry {
                network: err.is_network_conn_err(),
            };
        }
        match self.buffer.park(id, &self.failed).await {
            Ok(_) => {
                error!("calibrations: report {id} can never be uploaded; parked as failed/{id}.json: {err}");
                Outcome::Resolved
            }
            Err(park_err) => {
                error!("calibrations: failed to park report {id} ({err}): {park_err}");
                Outcome::Retry { network: false }
            }
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
                let result = self.buffer.enqueue(*report).await;
                respond(respond_to, result.map_err(CalibrationsErr::from), "enqueue");
            }
            Command::Len { respond_to } => respond(respond_to, self.buffer.len(), "len"),
            Command::Shutdown { respond_to } => {
                respond(respond_to, (), "shutdown");
                return Flow::Shutdown;
            }
        }
        Flow::Continue
    }
}

/// A report file that cannot be parsed can never be sent.
fn unreadable(e: ParseJSONErr) -> CalibrationsErr {
    CalibrationsErr::InvalidReportErr(InvalidReportErr {
        reason: format!("the report file cannot be parsed: {e}"),
        trace: trace!(),
    })
}

fn respond<T>(respond_to: oneshot::Sender<T>, value: T, op: &str) {
    if respond_to.send(value).is_err() {
        error!("calibrations: failed to send the {op} response");
    }
}

// ================================== HANDLE ======================================= //
/// Handle to the calibration uploader: one task that owns the calibration
/// buffer, buffers what intake sends it, and uploads reports to Miru one at a
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
            buffer: args.buffer,
            failed: args.failed,
            sender: args.sender,
            events: args.events,
            options: args.options,
            sleep_fn: args.sleep_fn,
            streak: Streak(0),
        };
        (Self { sender }, tokio::spawn(worker.run()))
    }

    /// Buffer `report` durably. A full buffer is the 507
    /// [`CalibrationsErr::BufferFullErr`].
    pub async fn enqueue(&self, report: Report) -> Result<Uuid, CalibrationsErr> {
        self.send_command("enqueue", |respond_to| Command::Enqueue {
            report: Box::new(report),
            respond_to,
        })
        .await?
    }

    /// The number of buffered reports, including one being uploaded.
    // an actor-round-tripping is_empty would be dead weight next to len()
    #[allow(clippy::len_without_is_empty)]
    pub async fn len(&self) -> Result<usize, CalibrationsErr> {
        self.send_command("len", |respond_to| Command::Len { respond_to })
            .await
    }

    /// Stop the uploader, dropping any in-flight upload. Buffered reports,
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
