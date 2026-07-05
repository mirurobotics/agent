// standard crates
use std::collections::HashMap;
use std::sync::Arc;

// internal crates
use crate::models::{UploadCollectionID, UploadRule};
pub use crate::scan::collection_scanner::{find_stable, Observation, StableFile};
use crate::scan::{collection_scanner::CollectionScanner, errors::*};
use crate::trace;

// external crates
use chrono::{DateTime, Utc};
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

// ======================== SINGLE-THREADED IMPLEMENTATION ========================= //
pub struct ScannerArgs {
    pub min_poll_interval_secs: i64,
    pub now_fn: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
}

/// Poll-based file watcher: it does NOT subscribe to OS/inotify filesystem
/// events. Instead each `scan()` re-enumerates the globbed files on a cadence
/// (one global `min_poll_interval_secs` shared by all collections) and applies a
/// size/mtime stability window to decide which files are stable. Newly-stable
/// files are deduped so each is reported exactly once.
///
/// State is partitioned per upload collection: the scanner is a thin wrapper over
/// a `HashMap<UploadCollectionID, CollectionScanner>`, each sub-scanner owning its
/// own rule/observations/dedupe/cadence.
pub struct SingleThreadScanner {
    collections: HashMap<UploadCollectionID, CollectionScanner>,
    min_poll_interval_secs: i64,
    now_fn: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
}

impl SingleThreadScanner {
    pub fn new(args: ScannerArgs) -> Self {
        Self {
            collections: HashMap::new(),
            min_poll_interval_secs: args.min_poll_interval_secs,
            now_fn: args.now_fn,
        }
    }

    /// Reconcile the active rule set into the per-collection sub-scanners,
    /// preserving each surviving collection's observation/dedupe/cadence state.
    ///
    /// Rules are folded to one-per-collection (last-in-the-vec wins) BEFORE any
    /// state is touched, collapsing the redeploy double-rule case. Collections no
    /// longer present are dropped; surviving collections get their rule replaced
    /// (state carried over) and new collections get a fresh sub-scanner (due on
    /// next scan).
    fn update_rules(&mut self, rules: Vec<UploadRule>) -> Result<(), UploadErr> {
        let mut folded: HashMap<UploadCollectionID, UploadRule> = HashMap::new();
        for rule in rules {
            folded.insert(rule.upload_collection_id.clone(), rule);
        }

        self.collections.retain(|cid, _| folded.contains_key(cid));

        for (cid, rule) in folded {
            match self.collections.get_mut(&cid) {
                Some(collection) => collection.set_rule(rule)?,
                None => {
                    self.collections.insert(cid, CollectionScanner::new(rule));
                }
            }
        }

        Ok(())
    }

    async fn scan(&mut self) -> Result<(), UploadErr> {
        // Each collection advances independently; order across collections is
        // unspecified. A collection's own cadence gates whether it runs this tick.
        for collection in self.collections.values_mut() {
            let now = (self.now_fn)();

            if !collection.is_due(now) {
                continue;
            }

            let stable = collection.scan(now).await;
            Self::emit_stable(stable);
            collection.reschedule(now, self.min_poll_interval_secs);
        }

        Ok(())
    }

    /// Placeholder sink: emit a log line per newly-stable file. `find_stable`
    /// already deduped against the collection's ledger, so every file
    /// here is newly stable. M3 replaces this with the digest + POST /uploads
    /// pipeline.
    fn emit_stable(stable: Vec<StableFile>) {
        for sf in stable {
            info!(
                file_path = %sf.path.display(),
                file_modified_at = %sf.modified_at,
                "upload candidate stable (M2 placeholder sink)"
            );
        }
    }
}

// ========================= MULTI-THREADED IMPLEMENTATION ========================= //
#[allow(async_fn_in_trait)]
pub trait ScannerExt {
    async fn update_rules(&self, rules: Vec<UploadRule>) -> Result<(), UploadErr>;
    async fn scan(&self) -> Result<(), UploadErr>;
    async fn shutdown(&self) -> Result<(), UploadErr>;
}

pub enum Command {
    UpdateRules {
        rules: Vec<UploadRule>,
        respond_to: oneshot::Sender<Result<(), UploadErr>>,
    },
    Scan {
        respond_to: oneshot::Sender<Result<(), UploadErr>>,
    },
    Shutdown {
        respond_to: oneshot::Sender<Result<(), UploadErr>>,
    },
    #[cfg(feature = "test")]
    GetRules {
        respond_to: oneshot::Sender<Result<Vec<UploadRule>, UploadErr>>,
    },
    /// Inspector: number of files recorded in the ledger. Lets actor
    /// tests observe stability/cadence/dedupe through the public handle without
    /// scraping logs (each newly-stable file is reported exactly once, so this
    /// count is a faithful proxy for "files that have crossed into stable").
    #[cfg(feature = "test")]
    GetLedgerCount {
        respond_to: oneshot::Sender<Result<usize, UploadErr>>,
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
                Command::UpdateRules { rules, respond_to } => {
                    dispatch!(
                        self.scanner.update_rules(rules),
                        respond_to,
                        "Actor failed to send update rules response"
                    );
                }
                Command::Scan { respond_to } => {
                    dispatch!(
                        self.scanner.scan().await,
                        respond_to,
                        "Actor failed to send scan response"
                    );
                }
                #[cfg(feature = "test")]
                Command::GetRules { respond_to } => {
                    let rules = self
                        .scanner
                        .collections
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
                        .collections
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
    ) -> Result<(Self, JoinHandle<()>), UploadErr> {
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
    ) -> Result<R, UploadErr> {
        let (send, recv) = oneshot::channel();
        self.sender.send(cmd(send)).await.map_err(|e| {
            UploadErr::SendActorMessageErr(SendActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })?;
        recv.await.map_err(|e| {
            UploadErr::ReceiveActorMessageErr(ReceiveActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })
    }

    #[cfg(feature = "test")]
    pub async fn get_rules(&self) -> Result<Vec<UploadRule>, UploadErr> {
        self.send_command(|tx| Command::GetRules { respond_to: tx })
            .await?
    }

    #[cfg(feature = "test")]
    pub async fn get_ledger_count(&self) -> Result<usize, UploadErr> {
        self.send_command(|tx| Command::GetLedgerCount { respond_to: tx })
            .await?
    }
}

impl ScannerExt for Scanner {
    async fn update_rules(&self, rules: Vec<UploadRule>) -> Result<(), UploadErr> {
        self.send_command(|tx| Command::UpdateRules {
            rules,
            respond_to: tx,
        })
        .await?
    }

    async fn scan(&self) -> Result<(), UploadErr> {
        self.send_command(|tx| Command::Scan { respond_to: tx })
            .await?
    }

    async fn shutdown(&self) -> Result<(), UploadErr> {
        self.send_command(|tx| Command::Shutdown { respond_to: tx })
            .await??;
        info!("Scanner shutdown complete");
        Ok(())
    }
}
