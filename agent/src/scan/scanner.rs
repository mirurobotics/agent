// standard crates
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

// internal crates
use crate::models::{Deployment, UploadCollectionID, UploadRule};
pub use crate::scan::collection::{Config, Observation, StableFile, State};
use crate::scan::{collection::CollectionScanner, errors::*};
use crate::trace;

// external crates
use chrono::{DateTime, Utc};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{debug, error, info};

// =============================== SCANNER EVENTS ================================== //
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanEvent {
    StableFile(StableFile),
}

const DEFAULT_BROADCAST_CAPACITY: usize = 256;

macro_rules! dispatch {
    ($op:expr, $respond_to:expr, $msg:expr) => {{
        let result = $op;
        if $respond_to.send(result).is_err() {
            error!($msg);
        }
    }};
}

// ======================== SINGLE-THREADED IMPLEMENTATION ========================= //
pub struct ScannerArgs {
    pub min_poll_interval_secs: i64,
    pub now_fn: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
    pub broadcast_capacity: usize,
}

impl Default for ScannerArgs {
    fn default() -> Self {
        Self {
            min_poll_interval_secs: 1,
            now_fn: Arc::new(Utc::now),
            broadcast_capacity: DEFAULT_BROADCAST_CAPACITY,
        }
    }
}

pub struct SingleThreadScanner {
    scanners: HashMap<UploadCollectionID, CollectionScanner>,
    deployed: HashSet<UploadCollectionID>,
    now_fn: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
    subscriber_tx: broadcast::Sender<ScanEvent>,
}

impl SingleThreadScanner {
    pub fn new(args: ScannerArgs) -> Self {
        let (subscriber_tx, _) = broadcast::channel(args.broadcast_capacity);
        Self {
            scanners: HashMap::new(),
            deployed: HashSet::new(),
            now_fn: args.now_fn,
            subscriber_tx,
        }
    }

    fn subscribe(&self) -> broadcast::Receiver<ScanEvent> {
        self.subscriber_tx.subscribe()
    }

    fn emit_stable_files(&self, stable_files: Vec<StableFile>) {
        for stable_file in stable_files {
            if let Err(e) = self.subscriber_tx.send(ScanEvent::StableFile(stable_file)) {
                debug!("no stable-file subscribers active: {e:?}");
            }
        }
    }

    fn set_configs(
        &mut self,
        deployment: Deployment,
        rules: Vec<UploadRule>,
    ) -> Result<(), ScanErr> {
        let mut deployed: HashSet<UploadCollectionID> = HashSet::new();
        for rule in rules.iter() {
            if deployed.contains(&rule.upload_collection_id) {
                return Err(ScanErr::DuplicateCollectionID(DuplicateCollectionID {
                    collection_id: rule.upload_collection_id.clone(),
                    trace: trace!(),
                }));
            }
            deployed.insert(rule.upload_collection_id.clone());
        }

        for rule in rules.iter() {
            let config = Config {
                deployment: deployment.clone(),
                rule: rule.clone(),
            };
            match self.scanners.get_mut(&rule.upload_collection_id) {
                Some(scanner) => scanner.set_config(config)?,
                None => {
                    self.scanners.insert(
                        rule.upload_collection_id.clone(),
                        CollectionScanner::new(config),
                    );
                }
            }
        }

        self.scanners.retain(|cid, _| deployed.contains(cid));

        Ok(())
    }

    async fn scan(&mut self) -> Result<(), ScanErr> {
        let mut stable_files = Vec::new();
        let mut inactive_colls = Vec::new();

        // evaluate the candidates for all scanners
        for (cid, scanner) in self.scanners.iter_mut() {
            let now = (self.now_fn)();
            stable_files.extend(scanner.evaluate_candidates(now).await?);

            // only deployed scanners discover candidates, other scanners continue
            // scanning their candidate pool until no candidates remain
            if self.deployed.contains(cid) {
                scanner.discover_candidates(now).await?;
                continue;
            } else if !scanner.has_candidates() {
                inactive_colls.push(cid.clone());
            }
        }

        // prune inactive collection scanners
        for cid in inactive_colls {
            self.scanners.remove(&cid);
        }

        self.emit_stable_files(stable_files);

        Ok(())
    }
}

// ========================= MULTI-THREADED IMPLEMENTATION ========================= //
#[allow(async_fn_in_trait)]
pub trait ScannerExt {
    async fn set_configs(&self, cfgs: Vec<Config>) -> Result<(), ScanErr>;
    async fn scan(&self) -> Result<(), ScanErr>;
    async fn subscribe(&self) -> Result<broadcast::Receiver<ScanEvent>, ScanErr>;
    async fn shutdown(&self) -> Result<(), ScanErr>;
}

pub enum Command {
    UpdateConfigs {
        cfgs: Vec<Config>,
        respond_to: oneshot::Sender<Result<(), ScanErr>>,
    },
    Scan {
        respond_to: oneshot::Sender<Result<(), ScanErr>>,
    },
    Subscribe {
        respond_to: oneshot::Sender<Result<broadcast::Receiver<ScanEvent>, ScanErr>>,
    },
    Shutdown {
        respond_to: oneshot::Sender<Result<(), ScanErr>>,
    },
    #[cfg(feature = "test")]
    GetRules {
        respond_to: oneshot::Sender<Result<Vec<UploadRule>, ScanErr>>,
    },
    /// Inspector: number of files recorded in the ledger. Lets actor
    /// tests observe stability/cadence/dedupe through the public handle without
    /// scraping logs (each newly-stable file is reported exactly once, so this
    /// count is a faithful proxy for "files that have crossed into stable").
    #[cfg(feature = "test")]
    GetLedgerCount {
        respond_to: oneshot::Sender<Result<usize, ScanErr>>,
    },
}

pub struct Worker {
    scanner: SingleThreadScanner,
    receiver: mpsc::Receiver<Command>,
}

impl Worker {
    pub fn new(scanner: SingleThreadScanner, receiver: mpsc::Receiver<Command>) -> Self {
        Self { scanner, receiver }
    }

    pub async fn run(mut self) {
        while let Some(cmd) = self.receiver.recv().await {
            match cmd {
                Command::Shutdown { respond_to } => {
                    if let Err(e) = respond_to.send(Ok(())) {
                        error!("Actor failed to send shutdown response: {:?}", e);
                    }
                    break;
                }
                Command::UpdateConfigs { cfgs, respond_to } => {
                    dispatch!(
                        self.scanner.set_configs(cfgs),
                        respond_to,
                        "Actor failed to send update configs response"
                    );
                }
                Command::Scan { respond_to } => {
                    dispatch!(
                        self.scanner.scan().await,
                        respond_to,
                        "Actor failed to send scan response"
                    );
                }
                Command::Subscribe { respond_to } => {
                    if respond_to.send(Ok(self.scanner.subscribe())).is_err() {
                        error!("Actor failed to send subscribe response");
                    }
                }
                #[cfg(feature = "test")]
                Command::GetRules { respond_to } => {
                    let rules = self
                        .scanner
                        .scanners
                        .values()
                        .map(|c| c.rule().clone())
                        .collect::<Vec<_>>();
                    if respond_to.send(Ok(rules)).is_err() {
                        error!("Actor failed to send get rules response");
                    }
                }
                #[cfg(feature = "test")]
                Command::GetLedgerCount { respond_to } => {
                    let count = self
                        .scanner
                        .scanners
                        .values()
                        .map(CollectionScanner::ledger_count)
                        .sum::<usize>();
                    if respond_to.send(Ok(count)).is_err() {
                        error!("Actor failed to send get ledger count response");
                    }
                }
            }
        }
    }
}

/// Cloneable handle to the poll-based file [`SingleThreadScanner`] actor. The
/// scanner polls the filesystem on a cadence (cadence-driven, not OS/inotify
/// event-driven) and reports newly-stable files exactly once.
#[derive(Debug)]
pub struct Scanner {
    sender: mpsc::Sender<Command>,
}

impl Scanner {
    pub fn spawn(
        buffer_size: usize,
        args: ScannerArgs,
    ) -> Result<(Self, JoinHandle<()>), ScanErr> {
        let (sender, receiver) = mpsc::channel(buffer_size);
        let worker = Worker {
            scanner: SingleThreadScanner::new(args),
            receiver,
        };
        let worker_handle = tokio::spawn(worker.run());
        Ok((Self { sender }, worker_handle))
    }

    pub fn new(sender: mpsc::Sender<Command>) -> Self {
        Self { sender }
    }

    async fn send_command<R>(
        &self,
        cmd: impl FnOnce(oneshot::Sender<R>) -> Command,
    ) -> Result<R, ScanErr> {
        let (send, recv) = oneshot::channel();
        self.sender.send(cmd(send)).await.map_err(|e| {
            ScanErr::SendActorMessageErr(SendActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })?;
        recv.await.map_err(|e| {
            ScanErr::ReceiveActorMessageErr(ReceiveActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })
    }

    #[cfg(feature = "test")]
    pub async fn get_rules(&self) -> Result<Vec<UploadRule>, ScanErr> {
        self.send_command(|tx| Command::GetRules { respond_to: tx })
            .await?
    }

    #[cfg(feature = "test")]
    pub async fn get_ledger_count(&self) -> Result<usize, ScanErr> {
        self.send_command(|tx| Command::GetLedgerCount { respond_to: tx })
            .await?
    }
}

impl ScannerExt for Scanner {
    async fn set_configs(&self, cfgs: Vec<Config>) -> Result<(), ScanErr> {
        self.send_command(|tx| Command::UpdateConfigs {
            cfgs,
            respond_to: tx,
        })
        .await?
    }

    async fn scan(&self) -> Result<(), ScanErr> {
        self.send_command(|tx| Command::Scan { respond_to: tx })
            .await?
    }

    async fn subscribe(&self) -> Result<broadcast::Receiver<ScanEvent>, ScanErr> {
        self.send_command(|tx| Command::Subscribe { respond_to: tx })
            .await?
    }

    async fn shutdown(&self) -> Result<(), ScanErr> {
        self.send_command(|tx| Command::Shutdown { respond_to: tx })
            .await??;
        info!("Scanner shutdown complete");
        Ok(())
    }
}
