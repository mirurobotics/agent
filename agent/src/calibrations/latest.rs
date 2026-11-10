// internal crates
use crate::calibrations::{
    errors::{CalibrationsErr, ReceiveActorMessageErr, SendActorMessageErr},
    latest_impl::{Filter, LatestStoreImpl},
    report::ReportMetadata,
};
use crate::disk::layout::LatestCalsLayout;
use crate::trace;

// external crates
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{error, info};
use uuid::Uuid;

macro_rules! dispatch {
    ($op:expr, $respond_to:expr, $msg:expr) => {{
        let result = $op;
        if $respond_to.send(result).is_err() {
            error!($msg);
        }
    }};
}

// ================================== WORKER ======================================= //
pub(crate) enum Command {
    Put {
        metadata: Box<ReportMetadata>,
        data: String,
        respond_to: oneshot::Sender<Result<(), CalibrationsErr>>,
    },
    List {
        filter: Filter,
        respond_to: oneshot::Sender<Result<Vec<ReportMetadata>, CalibrationsErr>>,
    },
    Content {
        report_id: Uuid,
        respond_to: oneshot::Sender<Result<String, CalibrationsErr>>,
    },
    Shutdown {
        respond_to: oneshot::Sender<Result<(), CalibrationsErr>>,
    },
}

pub(crate) struct Worker {
    store: LatestStoreImpl,
    receiver: mpsc::Receiver<Command>,
}

impl Worker {
    pub(crate) async fn run(mut self) {
        while let Some(cmd) = self.receiver.recv().await {
            match cmd {
                Command::Shutdown { respond_to } => {
                    if respond_to.send(Ok(())).is_err() {
                        error!("calibrations: failed to send the latest store's shutdown response");
                    }
                    break;
                }
                Command::Put {
                    metadata,
                    data,
                    respond_to,
                } => {
                    dispatch!(
                        self.store.put(*metadata, &data).await,
                        respond_to,
                        "calibrations: failed to send the latest store's put response"
                    );
                }
                Command::List { filter, respond_to } => {
                    dispatch!(
                        self.store.list(&filter).await,
                        respond_to,
                        "calibrations: failed to send the latest store's list response"
                    );
                }
                Command::Content {
                    report_id,
                    respond_to,
                } => {
                    dispatch!(
                        self.store.content(report_id).await,
                        respond_to,
                        "calibrations: failed to send the latest store's content response"
                    );
                }
            }
        }
    }
}

// ================================== HANDLE ======================================= //
/// Handle to the latest calibrations store's worker, which owns the store's
/// files and its index in memory, so it is the store's only writer. Clones
/// share the worker.
#[derive(Clone, Debug)]
pub struct LatestStore {
    sender: mpsc::Sender<Command>,
}

impl LatestStore {
    /// Open the store in `layout`, holding at most `capacity` keys, and start
    /// its worker. It first deletes the content no entry names, which a crash
    /// or a failed write or delete leaves.
    pub async fn spawn(
        buffer_size: usize,
        layout: LatestCalsLayout,
        capacity: usize,
    ) -> (Self, JoinHandle<()>) {
        let (sender, receiver) = mpsc::channel(buffer_size);
        let worker = Worker {
            store: LatestStoreImpl::open(layout, capacity).await,
            receiver,
        };
        (Self { sender }, tokio::spawn(worker.run()))
    }

    /// Make `metadata` its key's entry, with `data` as its content. A new key
    /// past the capacity is rejected rather than evicting another. A failure
    /// part way leaves at most content that no entry names, which the next
    /// open deletes.
    pub async fn put(&self, metadata: ReportMetadata, data: &str) -> Result<(), CalibrationsErr> {
        self.send_command(|respond_to| Command::Put {
            metadata: Box::new(metadata),
            data: data.to_string(),
            respond_to,
        })
        .await?
    }

    /// The entries `filter` matches, ordered by component, role and aspect.
    pub async fn list(&self, filter: &Filter) -> Result<Vec<ReportMetadata>, CalibrationsErr> {
        self.send_command(|respond_to| Command::List {
            filter: filter.clone(),
            respond_to,
        })
        .await?
    }

    /// The data of the entry for `report_id`. Not found unless an entry has
    /// that `report_id` and its content exists.
    pub async fn content(&self, report_id: Uuid) -> Result<String, CalibrationsErr> {
        self.send_command(|respond_to| Command::Content {
            report_id,
            respond_to,
        })
        .await?
    }

    /// Stop the worker. Calls through any clone then fail.
    pub async fn shutdown(&self) -> Result<(), CalibrationsErr> {
        info!("calibrations: shutting down latest store...");
        self.send_command(|respond_to| Command::Shutdown { respond_to })
            .await??;
        info!("calibrations: latest store shutdown complete");
        Ok(())
    }

    async fn send_command<R>(
        &self,
        cmd: impl FnOnce(oneshot::Sender<R>) -> Command,
    ) -> Result<R, CalibrationsErr> {
        let (send, recv) = oneshot::channel();
        self.sender.send(cmd(send)).await.map_err(|e| {
            CalibrationsErr::SendActorMessageErr(SendActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })?;
        recv.await.map_err(|e| {
            CalibrationsErr::ReceiveActorMessageErr(ReceiveActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })
    }
}
