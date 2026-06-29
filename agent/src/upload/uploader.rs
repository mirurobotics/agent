// standard crates
use std::cmp::max;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

// internal crates
use crate::models::{UploadRule, UploadRuleID};
use crate::trace;
use crate::upload::errors::*;

// external crates
use chrono::{DateTime, TimeDelta, Utc};
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

/// Per-file stability state used by the readiness state machine. A file is
/// "ready" once its (size, mtime) have been unchanged for at least
/// `stability_window_secs` since `stable_since`. Fields are public so unit
/// tests can construct/inspect observations against the pure `decide_ready`
/// seam directly.
pub struct FileObservation {
    pub size: u64,
    pub mtime: std::time::SystemTime,
    pub stable_since: DateTime<Utc>,
}

/// A file that has been determined ready for upload. Fields are public so unit
/// tests can assert against the pure `decide_ready` seam directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadyFile {
    pub path: PathBuf,
    pub modified_at: DateTime<Utc>,
}

// ======================== SINGLE-THREADED IMPLEMENTATION ========================= //
pub struct UploaderArgs {
    pub min_poll_interval_secs: i64,
    pub now_fn: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
}

pub struct SingleThreadUploader {
    rules: Vec<UploadRule>,
    next_scan_at: HashMap<UploadRuleID, DateTime<Utc>>,
    observations: HashMap<PathBuf, FileObservation>,
    already_reported: HashSet<PathBuf>,
    min_poll_interval_secs: i64,
    now_fn: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
}

impl SingleThreadUploader {
    pub fn new(args: UploaderArgs) -> Self {
        Self {
            rules: Vec::new(),
            next_scan_at: HashMap::new(),
            observations: HashMap::new(),
            already_reported: HashSet::new(),
            min_poll_interval_secs: args.min_poll_interval_secs,
            now_fn: args.now_fn,
        }
    }

    /// Replace the active rule set with `rules`, pruning per-rule cadence state
    /// for rules that are no longer present. Per-file observation/dedupe state is
    /// intentionally NOT pruned (lingering entries are harmless). New rules get no
    /// `next_scan_at` entry, so they are due on the next scan.
    fn update_rules(&mut self, rules: Vec<UploadRule>) {
        self.rules = rules;
        let present_ids: HashSet<&UploadRuleID> = self.rules.iter().map(|r| &r.id).collect();
        self.next_scan_at.retain(|id, _| present_ids.contains(id));
    }

    async fn scan(&mut self) -> Result<(), UploadErr> {
        for rule in &self.rules {
            let now = (self.now_fn)();

            let due = self
                .next_scan_at
                .get(&rule.id)
                .is_none_or(|next| *next <= now);
            if !due {
                continue;
            }

            // decide_ready does blocking fs I/O (glob enumeration + stat). Per the
            // Decision Log, run it under spawn_blocking to keep the runtime
            // responsive. The observations map is keyed by PathBuf across all
            // rules; we move it into the blocking task and take it back out
            // afterwards (alongside the newly-ready files) so decide_ready itself
            // stays a plain, unit-testable sync fn.
            let rule_clone = rule.clone();
            let moved_observations = std::mem::take(&mut self.observations);
            let moved_already_reported = std::mem::take(&mut self.already_reported);
            let (returned_observations, returned_already_reported, ready) =
                match tokio::task::spawn_blocking(move || {
                    let mut obs = moved_observations;
                    let mut reported = moved_already_reported;
                    let ready = decide_ready(&rule_clone, &mut obs, &mut reported, now);
                    (obs, reported, ready)
                })
                .await
                {
                    Ok(result) => result,
                    Err(e) => {
                        error!("uploads readiness task panicked: {e:?}");
                        (HashMap::new(), HashSet::new(), Vec::new())
                    }
                };
            self.observations = returned_observations;
            self.already_reported = returned_already_reported;

            // placeholder sink: emit a log line per newly-ready file. decide_ready
            // already deduped against already_reported, so every returned file is
            // newly ready. M3 replaces this with the digest + POST /uploads pipeline.
            for rf in ready {
                info!(
                    file_path = %rf.path.display(),
                    file_modified_at = %rf.modified_at,
                    "upload candidate ready (M2 placeholder sink)"
                );
            }

            let interval_secs = max(
                rule.source.poll_interval_secs as i64,
                self.min_poll_interval_secs,
            );
            self.next_scan_at
                .insert(rule.id.clone(), now + TimeDelta::seconds(interval_secs));
        }

        // prune next_scan_at entries for rules that are no longer present.
        let present_ids: HashSet<&UploadRuleID> = self.rules.iter().map(|r| &r.id).collect();
        self.next_scan_at.retain(|id, _| present_ids.contains(id));

        Ok(())
    }
}

/// Pure readiness decision for a single rule. Enumerates `rule.source.glob`,
/// stats each matched file, updates the per-file stability state in
/// `observations`, and returns the files that have been size/mtime-stable for
/// at least `stability_window_secs`.
///
/// This does BLOCKING filesystem I/O (glob walk + metadata) and is intended to
/// be called inside `tokio::task::spawn_blocking`. It is kept sync and pure
/// (state in/out via the `observations`/`already_reported` arguments) so it can
/// be unit-tested directly without a runtime.
///
/// Returns only NEWLY-ready files: a file crossing into "ready" is recorded in
/// `already_reported` and is never returned again, so callers can treat every
/// returned file as a once-per-file event.
pub fn decide_ready(
    rule: &UploadRule,
    observations: &mut HashMap<PathBuf, FileObservation>,
    already_reported: &mut HashSet<PathBuf>,
    now: DateTime<Utc>,
) -> Vec<ReadyFile> {
    let mut ready = Vec::new();

    let paths = match glob::glob(&rule.source.glob) {
        Ok(paths) => paths,
        Err(e) => {
            error!("invalid glob {:?}: {e}", rule.source.glob);
            return ready;
        }
    };

    for entry in paths {
        let path = match entry {
            Ok(path) => path,
            Err(_) => continue,
        };
        if !path.is_file() {
            continue;
        }

        let meta = match std::fs::metadata(&path) {
            Ok(meta) => meta,
            Err(_) => continue, // e.g. file deleted mid-scan
        };
        let size = meta.len();
        let mtime = match meta.modified() {
            Ok(mtime) => mtime,
            Err(_) => continue,
        };

        match observations.get(&path) {
            // size/mtime unchanged since last observation: the stability window
            // is still running; report once it has elapsed.
            Some(obs) if obs.size == size && obs.mtime == mtime => {
                // HOOK (M3): finalization-marker detection (MCAP footer / parquet
                // magic bytes) would gate readiness here in addition to size+mtime
                // stability. NOT implemented in M2.
                if now.signed_duration_since(obs.stable_since).num_seconds()
                    >= rule.source.stability_window_secs as i64
                    && !already_reported.contains(&path)
                {
                    ready.push(ReadyFile {
                        path: path.clone(),
                        modified_at: mtime.into(),
                    });
                    already_reported.insert(path.clone());
                }
            }
            // new file, or size/mtime changed since last observation: (re)start
            // the stability window and do NOT report.
            _ => {
                observations.insert(
                    path,
                    FileObservation {
                        size,
                        mtime,
                        stable_since: now,
                    },
                );
            }
        }
    }

    ready
}

// ========================= MULTI-THREADED IMPLEMENTATION ========================= //
#[allow(async_fn_in_trait)]
pub trait UploaderExt {
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
                Command::Scan { respond_to } => {
                    dispatch!(
                        self.uploader.scan().await,
                        respond_to,
                        "Actor failed to send scan response"
                    );
                }
                #[cfg(feature = "test")]
                Command::GetRules { respond_to } => {
                    if respond_to.send(Ok(self.uploader.rules.clone())).is_err() {
                        error!("Actor failed to send get rules response");
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
        args: UploaderArgs,
    ) -> Result<(Self, JoinHandle<()>), UploadErr> {
        let (sender, receiver) = mpsc::channel(buffer_size);
        let worker = Worker {
            uploader: SingleThreadUploader::new(args),
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
}

impl UploaderExt for Uploader {
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
        info!("Uploader shutdown complete");
        Ok(())
    }
}
