// standard crates
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::calibrations::{
    errors::{CalibrationsErr, ReceiveActorMessageErr, SendActorMessageErr},
    latest::{LatestCalibrations, LatestContent, LatestFilter, LatestStore},
    report::{QueueEntry, QueueSnapshotFile, Report},
    sender::Sender,
    uploader::{NextAttempt, Uploader, UploaderOptions},
};
use crate::filesys::Dir;
use crate::trace;

// external crates
use chrono::{DateTime, Utc};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{error, info};
use uuid::Uuid;

pub struct CalMngrArgs<S, F, N> {
    /// Where the queue persists; `None` keeps it in memory only.
    pub snapshot_file: Option<QueueSnapshotFile>,
    /// The directory holding one data file per queued report.
    pub reports: Dir,
    /// The latest calibrations store, which the worker writes beside the queue.
    pub latest: LatestStore,
    pub sender: Arc<S>,
    pub options: UploaderOptions,
    pub sleep_fn: F,
    /// The clock that report backoffs and ages are measured against.
    pub now_fn: N,
}

// ================================== WORKER ======================================= //
enum Command {
    Enqueue {
        report: Box<Report>,
        respond_to: oneshot::Sender<Result<(), CalibrationsErr>>,
    },
    Len {
        respond_to: oneshot::Sender<usize>,
    },
    ListLatest {
        filter: LatestFilter,
        respond_to: oneshot::Sender<Result<LatestCalibrations, CalibrationsErr>>,
    },
    LatestContent {
        report_id: Uuid,
        respond_to: oneshot::Sender<Result<LatestContent, CalibrationsErr>>,
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
    uploader: Uploader<S, N>,
    latest: LatestStore,
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
        self.uploader.prune_orphans().await;
        loop {
            let flow = match self.uploader.next_attempt().await {
                NextAttempt::Ready(entry) => self.run_attempt(entry).await,
                NextAttempt::Empty => self.idle().await,
                NextAttempt::Backoff(wait) => self.back_off(wait).await,
            };
            if let Flow::Shutdown = flow {
                break;
            }
        }
        info!("calibrations: manager shutdown complete");
    }

    /// Upload the report queued as `entry` and settle it. A shutdown
    /// mid-upload leaves the report queued, to be re-sent on the next boot.
    /// A sent report is marked uploaded before it is dequeued, so a crash in
    /// between re-sends it rather than losing the mark.
    async fn run_attempt(&mut self, entry: QueueEntry) -> Flow {
        let upload = self.uploader.attempt(&entry);
        let Some(result) = self.serve_until(upload).await else {
            return Flow::Shutdown;
        };
        if result.is_ok() {
            self.latest.mark_uploaded(entry.job.report_id).await;
        }
        self.uploader.finish_attempt(entry, result).await;
        Flow::Continue
    }

    /// Queue `report`, then write it to the latest store. A failed store
    /// write removes the queued report, so on error nothing is queued or
    /// stored. Size goes first, so an oversized report is a 413 even when the
    /// store is full.
    async fn enqueue(&mut self, report: Report) -> Result<(), CalibrationsErr> {
        self.uploader.verify_size(report.data.len() as u64)?;
        self.latest.verify_capacity(&report.metadata).await?;
        let id = self.uploader.enqueue(&report).await?;
        if let Err(e) = self.latest.put(report.metadata, &report.data).await {
            self.uploader.remove(id).await;
            return Err(e);
        }
        Ok(())
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

    /// Wait `wait` for the earliest report to come due, or serve one command,
    /// whichever comes first, as `data_uploads::upload` does. Either way the
    /// run loop then re-evaluates the queue, so a new report is considered at
    /// once. Commands take priority over a sleep that ends at the same time.
    async fn back_off(&mut self, wait: Duration) -> Flow {
        let sleep = (self.sleep_fn)(wait);
        tokio::select! {
            biased;
            cmd = self.receiver.recv() => self.handle(cmd).await,
            () = sleep => Flow::Continue,
        }
    }

    /// Wait for a command with nothing ready to send.
    async fn idle(&mut self) -> Flow {
        let cmd = self.receiver.recv().await;
        self.handle(cmd).await
    }

    /// Serve `cmd`. Every handle dropping counts as a shutdown.
    async fn handle(&mut self, cmd: Option<Command>) -> Flow {
        let Some(cmd) = cmd else {
            return Flow::Shutdown;
        };
        match cmd {
            Command::Enqueue { report, respond_to } => {
                let result = self.enqueue(*report).await;
                respond(respond_to, result, "enqueue");
            }
            Command::Len { respond_to } => respond(respond_to, self.uploader.len(), "len"),
            Command::ListLatest { filter, respond_to } => {
                let queued = self.uploader.queued_report_ids();
                let latest = self.latest.list(&filter, &queued).await;
                respond(respond_to, latest, "list latest");
            }
            Command::LatestContent {
                report_id,
                respond_to,
            } => {
                let content = self.latest.content(report_id).await;
                respond(respond_to, content, "latest content");
            }
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
/// Handle to the calibration manager: one task that owns the calibration
/// queue and the latest store, queues what intake sends it, and uploads
/// reports to Miru one at a time, oldest due first, retrying a failed report at
/// the tail after its own backoff. It keeps serving intake and latest reads
/// during an upload or a backoff.
#[derive(Debug)]
pub struct CalibrationManager {
    sender: mpsc::Sender<Command>,
}

impl CalibrationManager {
    pub fn spawn<S, F, Fut, N>(
        buffer_size: usize,
        args: CalMngrArgs<S, F, N>,
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
            uploader: Uploader::new(
                args.sender,
                args.snapshot_file,
                args.reports,
                args.options,
                args.now_fn,
            ),
            latest: args.latest,
            sleep_fn: args.sleep_fn,
        };
        (Self { sender }, tokio::spawn(worker.run()))
    }

    /// Queue `report` durably and write it to the latest store. A full queue
    /// is the 507 [`CalibrationsErr::QueueFullErr`], a full store the 507
    /// [`CalibrationsErr::StoreFullErr`], and a report over the size cap the
    /// 413 [`CalibrationsErr::TooLargeErr`].
    pub async fn report(&self, report: Report) -> Result<(), CalibrationsErr> {
        self.send_command("enqueue", |respond_to| Command::Enqueue {
            report: Box::new(report),
            respond_to,
        })
        .await?
    }

    /// The number of queued reports, including one being uploaded.
    pub async fn num_queued_uploads(&self) -> Result<usize, CalibrationsErr> {
        self.send_command("len", |respond_to| Command::Len { respond_to })
            .await
    }

    /// The latest entries `filter` matches.
    pub async fn list_latest(
        &self,
        filter: LatestFilter,
    ) -> Result<LatestCalibrations, CalibrationsErr> {
        self.send_command("list latest", |respond_to| Command::ListLatest {
            filter,
            respond_to,
        })
        .await?
    }

    /// The content of the latest entry for `report_id`.
    pub async fn latest_content(&self, report_id: Uuid) -> Result<LatestContent, CalibrationsErr> {
        self.send_command("latest content", |respond_to| Command::LatestContent {
            report_id,
            respond_to,
        })
        .await?
    }

    /// Stop the manager, dropping any in-flight upload. Queued reports,
    /// including the interrupted one, stay on disk for the next boot.
    pub async fn shutdown(&self) -> Result<(), CalibrationsErr> {
        info!("calibrations: shutting down manager...");
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
            error!("calibrations: failed to send the {op} command to the manager: {e}");
            CalibrationsErr::SendActorMessageErr(SendActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })?;
        recv.await.map_err(|e| {
            error!("calibrations: failed to receive the {op} response from the manager: {e}");
            CalibrationsErr::ReceiveActorMessageErr(ReceiveActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })
    }
}
