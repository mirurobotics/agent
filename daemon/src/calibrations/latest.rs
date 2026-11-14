// internal crates
use crate::calibrations::{
    errors::{CalibrationsErr, ReceiveActorMessageErr, SendActorMessageErr},
    latest_impl::{Filter, LatestStoreImpl},
    report::{Report, ReportMetadata},
};
use crate::disk::layout::LatestCalsLayout;
use crate::trace;

// external crates
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{error, info};
use uuid::Uuid;

/// The most keys the store holds in the agent.
pub const DEFAULT_CAPACITY: usize = 1000;

// ================================== WORKER ======================================= //
enum Command {
    Put {
        metadata: Box<ReportMetadata>,
        data: String,
        respond_to: oneshot::Sender<Result<(), CalibrationsErr>>,
    },
    PutIfNewer {
        metadata: Box<ReportMetadata>,
        data: String,
        respond_to: oneshot::Sender<Result<bool, CalibrationsErr>>,
    },
    List {
        filter: Filter,
        respond_to: oneshot::Sender<Result<Vec<ReportMetadata>, CalibrationsErr>>,
    },
    Get {
        report_id: Uuid,
        respond_to: oneshot::Sender<Result<Report, CalibrationsErr>>,
    },
    Shutdown {
        respond_to: oneshot::Sender<()>,
    },
}

struct Worker {
    receiver: mpsc::Receiver<Command>,
    store: LatestStoreImpl,
}

impl Worker {
    /// Serve commands one at a time. Every handle dropping counts as a
    /// shutdown.
    async fn run(mut self) {
        self.store.prune_orphaned_contents().await;
        while let Some(cmd) = self.receiver.recv().await {
            match cmd {
                Command::Put {
                    metadata,
                    data,
                    respond_to,
                } => {
                    let result = self.store.put(*metadata, &data).await;
                    respond(respond_to, result, "put");
                }
                Command::PutIfNewer {
                    metadata,
                    data,
                    respond_to,
                } => {
                    let result = self.store.put_if_newer(*metadata, &data).await;
                    respond(respond_to, result, "put if newer");
                }
                Command::List { filter, respond_to } => {
                    let result = self.store.list(&filter).await;
                    respond(respond_to, result, "list");
                }
                Command::Get {
                    report_id,
                    respond_to,
                } => {
                    let result = self.store.get(report_id).await;
                    respond(respond_to, result, "get");
                }
                Command::Shutdown { respond_to } => {
                    respond(respond_to, (), "shutdown");
                    break;
                }
            }
        }
        info!("calibrations: latest store shutdown complete");
    }
}

fn respond<T>(respond_to: oneshot::Sender<T>, value: T, op: &str) {
    if respond_to.send(value).is_err() {
        error!("calibrations: failed to send the latest store's {op} response");
    }
}

// ================================== HANDLE ======================================= //
/// Handle to the latest calibrations store's worker, which owns the store's
/// files, so it is the store's only writer. Clones share the worker.
#[derive(Clone, Debug)]
pub struct LatestStore {
    sender: mpsc::Sender<Command>,
}

impl LatestStore {
    /// Start the worker for the store in `layout`, holding at most `capacity`
    /// keys. Before serving a command it deletes the content no entry names.
    pub fn spawn(
        buffer_size: usize,
        layout: LatestCalsLayout,
        capacity: usize,
    ) -> (Self, JoinHandle<()>) {
        let (sender, receiver) = mpsc::channel(buffer_size);
        let worker = Worker {
            receiver,
            store: LatestStoreImpl::new(layout, capacity),
        };
        (Self { sender }, tokio::spawn(worker.run()))
    }

    /// Make `metadata` its key's entry, with `data` as its content. A new key
    /// past the capacity is rejected rather than evicting another. A failure
    /// part way leaves at most content that no entry names, which the next
    /// spawn deletes.
    pub async fn put(&self, metadata: ReportMetadata, data: &str) -> Result<(), CalibrationsErr> {
        self.send_command("put", |respond_to| Command::Put {
            metadata: Box::new(metadata),
            data: data.to_string(),
            respond_to,
        })
        .await?
    }

    /// Make `metadata` its key's entry, as [`Self::put`] does, unless that
    /// entry was reported at the same time or later. Returns whether it did.
    pub async fn put_if_newer(
        &self,
        metadata: ReportMetadata,
        data: &str,
    ) -> Result<bool, CalibrationsErr> {
        self.send_command("put if newer", |respond_to| Command::PutIfNewer {
            metadata: Box::new(metadata),
            data: data.to_string(),
            respond_to,
        })
        .await?
    }

    /// The entries `filter` matches, ordered by component, role and aspect.
    pub async fn list(&self, filter: &Filter) -> Result<Vec<ReportMetadata>, CalibrationsErr> {
        self.send_command("list", |respond_to| Command::List {
            filter: filter.clone(),
            respond_to,
        })
        .await?
    }

    /// The entry for `report_id`, with its data. Not found unless an entry has
    /// that `report_id` and its content exists.
    pub async fn get(&self, report_id: Uuid) -> Result<Report, CalibrationsErr> {
        self.send_command("get", |respond_to| Command::Get {
            report_id,
            respond_to,
        })
        .await?
    }

    /// Stop the worker. Calls through any clone then fail.
    pub async fn shutdown(&self) -> Result<(), CalibrationsErr> {
        info!("calibrations: shutting down latest store...");
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
            error!("calibrations: failed to send the {op} command to the latest store: {e}");
            CalibrationsErr::SendActorMessageErr(SendActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })?;
        recv.await.map_err(|e| {
            error!("calibrations: failed to receive the {op} response from the latest store: {e}");
            CalibrationsErr::ReceiveActorMessageErr(ReceiveActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })
    }
}
