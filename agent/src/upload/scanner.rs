// standard crates
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

// internal crates
use crate::models::{UploadCollectionID, UploadRule};
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

/// Per-file stability state used by the readiness state machine. A file is "ready" once
/// its (size, mtime) have been unchanged for at least `stability_window_secs` since
/// `stable_since`. Fields are public so unit tests can construct/inspect observations
/// against the pure `decide_ready` seam directly.
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
pub struct ScannerArgs {
    pub min_poll_interval_secs: i64,
    pub now_fn: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
}

/// Owned (non-actor) sub-scanner for a single upload collection. Holds exactly
/// one active `UploadRule` plus that collection's own observation/dedupe/cadence
/// state. This is the structural home of what used to be global scanner state:
/// each collection now advances its readiness state machine independently.
struct CollectionScanner {
    rule: UploadRule,
    observations: HashMap<PathBuf, FileObservation>,
    already_reported: HashSet<PathBuf>,
    next_scan_at: Option<DateTime<Utc>>,
}

impl CollectionScanner {
    /// New sub-scanner with empty state and no recorded next-scan time, so it is
    /// due on its first scan.
    fn new(rule: UploadRule) -> Self {
        Self {
            rule,
            observations: HashMap::new(),
            already_reported: HashSet::new(),
            next_scan_at: None,
        }
    }

    /// Replace only the active rule, carrying over observation/dedupe/cadence
    /// state. This is the whole point of the collection-keyed partition: a rule
    /// redeploy (new rule id / digest for the same collection) preserves the
    /// in-flight stability windows and dedupe set for that collection.
    fn set_rule(&mut self, rule: UploadRule) {
        self.rule = rule;
    }

    /// Per-collection cadence due check: due when it has no recorded next-scan
    /// time (freshly added) or its next-scan time has arrived.
    fn is_due(&self, now: DateTime<Utc>) -> bool {
        self.next_scan_at.is_none_or(|next| next <= now)
    }

    /// Schedule this collection's next scan one global interval out.
    fn reschedule(&mut self, now: DateTime<Utc>, min_poll_interval_secs: i64) {
        self.next_scan_at = Some(now + TimeDelta::seconds(min_poll_interval_secs));
    }

    /// Run the readiness decision for this collection's rule under
    /// `spawn_blocking`.
    ///
    /// `decide_ready` does blocking fs I/O (glob enumeration + stat). Per the
    /// Decision Log, run it under spawn_blocking to keep the runtime responsive.
    /// Because `spawn_blocking` needs `'static`, we move this collection's
    /// observations map (and dedupe set) into the blocking task and take them
    /// back out afterwards (alongside the newly-ready files) so decide_ready
    /// itself stays a plain, unit-testable sync fn. If the task panics we log and
    /// restore empty state.
    async fn run_readiness(&mut self, now: DateTime<Utc>) -> Vec<ReadyFile> {
        let moved_observations = std::mem::take(&mut self.observations);
        let moved_already_reported = std::mem::take(&mut self.already_reported);
        let rule = self.rule.clone();
        let (returned_observations, returned_already_reported, ready) =
            match tokio::task::spawn_blocking(move || {
                let mut obs = moved_observations;
                let mut reported = moved_already_reported;
                let ready = decide_ready(&rule, &mut obs, &mut reported, now);
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
        ready
    }
}

/// Poll-based file watcher: it does NOT subscribe to OS/inotify filesystem
/// events. Instead each `scan()` re-enumerates the globbed files on a cadence
/// (one global `min_poll_interval_secs` shared by all collections) and applies a
/// size/mtime stability window to decide which files are ready. Newly-ready
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
    fn update_rules(&mut self, rules: Vec<UploadRule>) {
        let mut folded: HashMap<UploadCollectionID, UploadRule> = HashMap::new();
        for rule in rules {
            folded.insert(rule.upload_collection_id.clone(), rule);
        }

        self.collections.retain(|cid, _| folded.contains_key(cid));

        for (cid, rule) in folded {
            match self.collections.get_mut(&cid) {
                Some(collection) => collection.set_rule(rule),
                None => {
                    self.collections.insert(cid, CollectionScanner::new(rule));
                }
            }
        }
    }

    async fn scan(&mut self) -> Result<(), UploadErr> {
        // Each collection advances independently; order across collections is
        // unspecified. A collection's own cadence gates whether it runs this tick.
        for collection in self.collections.values_mut() {
            let now = (self.now_fn)();

            if !collection.is_due(now) {
                continue;
            }

            let ready = collection.run_readiness(now).await;
            Self::emit_ready(ready);
            collection.reschedule(now, self.min_poll_interval_secs);
        }

        Ok(())
    }

    /// Placeholder sink: emit a log line per newly-ready file. `decide_ready`
    /// already deduped against the collection's `already_reported`, so every file
    /// here is newly ready. M3 replaces this with the digest + POST /uploads
    /// pipeline.
    fn emit_ready(ready: Vec<ReadyFile>) {
        for rf in ready {
            info!(
                file_path = %rf.path.display(),
                file_modified_at = %rf.modified_at,
                "upload candidate ready (M2 placeholder sink)"
            );
        }
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

    for (path, size, mtime) in stat_glob_matches(&rule.source.glob) {
        if let Some(rf) = decide_file(
            path,
            size,
            mtime,
            rule.source.stability_window_secs,
            observations,
            already_reported,
            now,
        ) {
            ready.push(rf);
        }
    }

    ready
}

/// Enumerate `glob` and stat every matched existing regular file, yielding
/// `(path, size, mtime)` per file. An invalid glob is logged and yields nothing;
/// non-files and entries whose metadata/mtime cannot be read are skipped (e.g. a
/// file deleted mid-scan).
fn stat_glob_matches(glob: &str) -> Vec<(PathBuf, u64, std::time::SystemTime)> {
    let mut matches = Vec::new();

    let paths = match glob::glob(glob) {
        Ok(paths) => paths,
        Err(e) => {
            error!("invalid glob {:?}: {e}", glob);
            return matches;
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

        matches.push((path, size, mtime));
    }

    matches
}

/// Advance the stability window for one file and decide whether it is newly
/// ready. When (size, mtime) are unchanged since the last observation and the
/// window has elapsed, the file is reported once (recorded in `already_reported`
/// and returned). Otherwise the window is (re)started by inserting a fresh
/// observation and nothing is returned.
fn decide_file(
    path: PathBuf,
    size: u64,
    mtime: std::time::SystemTime,
    stability_window_secs: i32,
    observations: &mut HashMap<PathBuf, FileObservation>,
    already_reported: &mut HashSet<PathBuf>,
    now: DateTime<Utc>,
) -> Option<ReadyFile> {
    match observations.get(&path) {
        // size/mtime unchanged since last observation: the stability window
        // is still running; report once it has elapsed.
        Some(obs) if obs.size == size && obs.mtime == mtime => {
            // HOOK (M3): finalization-marker detection (MCAP footer / parquet
            // magic bytes) would gate readiness here in addition to size+mtime
            // stability. NOT implemented in M2.
            if now.signed_duration_since(obs.stable_since).num_seconds()
                >= stability_window_secs as i64
                && !already_reported.contains(&path)
            {
                already_reported.insert(path.clone());
                return Some(ReadyFile {
                    path,
                    modified_at: mtime.into(),
                });
            }
            None
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
            None
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
    /// Inspector: number of files recorded in `already_reported`. Lets actor
    /// tests observe readiness/cadence/dedupe through the public handle without
    /// scraping logs (each newly-ready file is reported exactly once, so this
    /// count is a faithful proxy for "files that have crossed into ready").
    #[cfg(feature = "test")]
    GetReportedCount {
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
                    self.scanner.update_rules(rules);
                    if let Err(e) = respond_to.send(Ok(())) {
                        error!("Actor failed to send update rules response: {:?}", e);
                    }
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
                        .map(|c| c.rule.clone())
                        .collect::<Vec<_>>();
                    if respond_to.send(Ok(rules)).is_err() {
                        error!("Actor failed to send get rules response");
                    }
                }
                #[cfg(feature = "test")]
                Command::GetReportedCount { respond_to } => {
                    let count = self
                        .scanner
                        .collections
                        .values()
                        .map(|c| c.already_reported.len())
                        .sum::<usize>();
                    if respond_to.send(Ok(count)).is_err() {
                        error!("Actor failed to send get reported count response");
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
    pub async fn get_reported_count(&self) -> Result<usize, UploadErr> {
        self.send_command(|tx| Command::GetReportedCount { respond_to: tx })
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
