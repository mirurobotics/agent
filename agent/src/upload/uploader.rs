// standard crates
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

// internal crates
use crate::models;
use crate::trace;
use crate::upload::{discovery, errors::*};

// external crates
use chrono::{DateTime, TimeDelta, Utc};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{debug, error, info};

macro_rules! dispatch {
    ($op:expr, $respond_to:expr, $msg:expr) => {{
        let result = $op;
        if $respond_to.send(result).is_err() {
            error!($msg);
        }
    }};
}

// ================================= OPTIONS ======================================= //
#[derive(Debug, Clone)]
pub struct Options {
    /// The global minimum interval between filesystem scans. Scheduling is
    /// deliberately not per-rule: the contract carries no per-rule poll
    /// interval, so the uploader owns a single cadence for the whole rule set.
    pub poll_interval_secs: i64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            poll_interval_secs: 60,
        }
    }
}

// ================================ SCAN OUTCOME =================================== //
/// A file whose size and mtime have quiesced for its stability window.
#[derive(Clone, Debug, PartialEq)]
pub struct ReadyFile {
    pub path: PathBuf,
    pub modified_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ScanOutcome {
    /// The global minimum poll interval has not elapsed since the last scan.
    NotDue,
    /// A scan ran; carries the files that became ready during this scan.
    Completed(Vec<ReadyFile>),
}

// ======================== SINGLE-THREADED IMPLEMENTATION ========================= //
type NowFn = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

pub struct SingleThreadUploader {
    options: Options,
    now_fn: NowFn,

    // uploader state (in-memory only — no storage dependency)
    rules: Vec<models::UploadRule>,
    observations: HashMap<PathBuf, discovery::Observation>,
    reported: HashSet<PathBuf>,
    last_scanned_at: DateTime<Utc>,
}

impl SingleThreadUploader {
    pub fn new(options: Options) -> Self {
        Self {
            options,
            now_fn: Arc::new(Utc::now),
            rules: Vec::new(),
            observations: HashMap::new(),
            reported: HashSet::new(),
            last_scanned_at: DateTime::<Utc>::UNIX_EPOCH,
        }
    }

    fn update_rules(&mut self, rules: Vec<models::UploadRule>) {
        debug!("uploader active rule set replaced ({} rules)", rules.len());
        self.rules = rules;
    }

    fn get_rules(&self) -> Vec<models::UploadRule> {
        self.rules.clone()
    }

    #[cfg(feature = "test")]
    fn set_now_fn(&mut self, now_fn: NowFn) {
        self.now_fn = now_fn;
    }

    async fn scan(&mut self) -> Result<ScanOutcome, UploadErr> {
        let now = (self.now_fn)();
        let interval = TimeDelta::seconds(self.options.poll_interval_secs.max(0));
        if now.signed_duration_since(self.last_scanned_at) < interval {
            return Ok(ScanOutcome::NotDue);
        }
        self.last_scanned_at = now;

        if self.rules.is_empty() {
            self.observations.clear();
            return Ok(ScanOutcome::Completed(Vec::new()));
        }

        let candidates = self.discover().await?;
        let ready = self.reconcile(candidates, now);
        for file in &ready {
            emit_ready(file);
        }
        Ok(ScanOutcome::Completed(ready))
    }

    /// Enumerates matching files off the async runtime — the glob walk and
    /// stats are blocking filesystem work.
    async fn discover(&self) -> Result<Vec<discovery::Candidate>, UploadErr> {
        let rules = self.rules.clone();
        tokio::task::spawn_blocking(move || discovery::discover_blocking(&rules))
            .await
            .map_err(|e| {
                UploadErr::JoinDiscoveryTaskErr(JoinDiscoveryTaskErr {
                    source: Box::new(e),
                    trace: trace!(),
                })
            })
    }

    /// Folds the scan's candidates into the per-file observation state,
    /// dropping state for files that no longer match, and returns the files
    /// that became ready this scan (each path is reported at most once).
    fn reconcile(
        &mut self,
        candidates: Vec<discovery::Candidate>,
        now: DateTime<Utc>,
    ) -> Vec<ReadyFile> {
        let mut ready = Vec::new();
        let mut observations = HashMap::with_capacity(candidates.len());
        for candidate in candidates {
            let observation = discovery::observe(
                self.observations.get(&candidate.path),
                candidate.size,
                candidate.modified_at,
                now,
            );
            let is_ready = discovery::is_ready(&observation, candidate.stability_window_secs, now)
                && discovery::passes_finalization_markers(&candidate.path);
            if is_ready && self.reported.insert(candidate.path.clone()) {
                ready.push(ReadyFile {
                    path: candidate.path.clone(),
                    modified_at: observation.modified_at,
                });
            }
            observations.insert(candidate.path, observation);
        }
        self.observations = observations;
        ready
    }
}

/// Placeholder sink for newly-ready files. M3 replaces this log line with the
/// mint (`POST /uploads`) → presigned `PUT` → confirm pipeline.
fn emit_ready(file: &ReadyFile) {
    info!(
        "file ready for upload: '{}' (modified at {})",
        file.path.display(),
        file.modified_at
    );
}

// ========================= MULTI-THREADED IMPLEMENTATION ========================= //
#[allow(async_fn_in_trait)]
pub trait UploaderExt {
    async fn shutdown(&self) -> Result<(), UploadErr>;
    async fn update_rules(&self, rules: Vec<models::UploadRule>) -> Result<(), UploadErr>;
    async fn get_rules(&self) -> Result<Vec<models::UploadRule>, UploadErr>;
    async fn scan(&self) -> Result<ScanOutcome, UploadErr>;
}

pub enum Command {
    Shutdown {
        respond_to: oneshot::Sender<Result<(), UploadErr>>,
    },
    UpdateRules {
        rules: Vec<models::UploadRule>,
        respond_to: oneshot::Sender<Result<(), UploadErr>>,
    },
    GetRules {
        respond_to: oneshot::Sender<Result<Vec<models::UploadRule>, UploadErr>>,
    },
    Scan {
        respond_to: oneshot::Sender<Result<ScanOutcome, UploadErr>>,
    },
    #[cfg(feature = "test")]
    SetNowFn {
        now_fn: NowFn,
        respond_to: oneshot::Sender<Result<(), UploadErr>>,
    },
}

pub struct Worker {
    uploader: SingleThreadUploader,
    receiver: mpsc::Receiver<Command>,
}

impl Worker {
    pub fn new(uploader: SingleThreadUploader, receiver: mpsc::Receiver<Command>) -> Self {
        Self { uploader, receiver }
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
                    self.uploader.update_rules(rules);
                    if let Err(e) = respond_to.send(Ok(())) {
                        error!("Actor failed to send update rules response: {:?}", e);
                    }
                }
                Command::GetRules { respond_to } => {
                    dispatch!(
                        Ok(self.uploader.get_rules()),
                        respond_to,
                        "Actor failed to send get rules response"
                    );
                }
                Command::Scan { respond_to } => {
                    dispatch!(
                        self.uploader.scan().await,
                        respond_to,
                        "Actor failed to send scan response"
                    );
                }
                #[cfg(feature = "test")]
                Command::SetNowFn { now_fn, respond_to } => {
                    self.uploader.set_now_fn(now_fn);
                    if let Err(e) = respond_to.send(Ok(())) {
                        error!("Actor failed to send set now fn response: {:?}", e);
                    }
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct Uploader {
    sender: mpsc::Sender<Command>,
}

impl Uploader {
    pub fn spawn(
        buffer_size: usize,
        options: Options,
    ) -> Result<(Self, JoinHandle<()>), UploadErr> {
        let (sender, receiver) = mpsc::channel(buffer_size);
        let worker = Worker {
            uploader: SingleThreadUploader::new(options),
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
    pub async fn set_now_fn(
        &self,
        now_fn: impl Fn() -> DateTime<Utc> + Send + Sync + 'static,
    ) -> Result<(), UploadErr> {
        self.send_command(|tx| Command::SetNowFn {
            now_fn: Arc::new(now_fn),
            respond_to: tx,
        })
        .await?
    }
}

impl UploaderExt for Uploader {
    async fn shutdown(&self) -> Result<(), UploadErr> {
        self.send_command(|tx| Command::Shutdown { respond_to: tx })
            .await??;
        info!("Uploader shutdown complete");
        Ok(())
    }

    async fn update_rules(&self, rules: Vec<models::UploadRule>) -> Result<(), UploadErr> {
        self.send_command(|tx| Command::UpdateRules {
            rules,
            respond_to: tx,
        })
        .await?
    }

    async fn get_rules(&self) -> Result<Vec<models::UploadRule>, UploadErr> {
        self.send_command(|tx| Command::GetRules { respond_to: tx })
            .await?
    }

    async fn scan(&self) -> Result<ScanOutcome, UploadErr> {
        self.send_command(|tx| Command::Scan { respond_to: tx })
            .await?
    }
}
