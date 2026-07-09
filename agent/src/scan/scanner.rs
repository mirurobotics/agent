// standard crates
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

// internal crates
use crate::models::{Deployment, UploadCollectionID, UploadRule};
pub use crate::scan::state::{Config, StableFile};
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
    pub now_fn: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
    pub broadcast_capacity: usize,
}

impl Default for ScannerArgs {
    fn default() -> Self {
        Self {
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

    #[cfg(feature = "test")]
    async fn get_rules(&self) -> Result<Vec<UploadRule>, ScanErr> {
        let rules = self
            .scanners
            .values()
            .map(|c| c.rule().clone())
            .collect::<Vec<_>>();
        Ok(rules)
    }

    #[cfg(feature = "test")]
    async fn get_ledger_count(&self) -> Result<usize, ScanErr> {
        let count = self
            .scanners
            .values()
            .map(CollectionScanner::ledger_count)
            .sum::<usize>();
        Ok(count)
    }

    async fn clear_rules(&mut self) -> Result<(), ScanErr> {
        self.deployed.clear();
        Ok(())
    }

    async fn update_rules(
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

        let now = (self.now_fn)();
        for rule in rules.iter() {
            let config = Config {
                deployment: deployment.clone(),
                rule: rule.clone(),
            };
            match self.scanners.get_mut(&rule.upload_collection_id) {
                Some(scanner) => scanner.update_config(config, now).await?,
                None => {
                    self.scanners.insert(
                        rule.upload_collection_id.clone(),
                        CollectionScanner::new(config, now).await?,
                    );
                }
            }
        }

        self.deployed = deployed;

        Ok(())
    }

    async fn scan(&mut self) -> Result<(), ScanErr> {
        let mut stable_files = Vec::new();
        let mut inactive_colls = Vec::new();

        // one timestamp for the whole tick so every sub-scanner evaluates against
        // the same `now`.
        let now = (self.now_fn)();

        // evaluate the candidates for all scanners
        for (cid, scanner) in self.scanners.iter_mut() {
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

    async fn prune(&mut self, before: DateTime<Utc>) -> Result<(), ScanErr> {
        for (_, scanner) in self.scanners.iter_mut() {
            scanner.prune_ledger(before)?;
        }
        Ok(())
    }
}

// ========================= MULTI-THREADED IMPLEMENTATION ========================= //
#[allow(async_fn_in_trait)]
pub trait ScannerExt {
    async fn clear_rules(&self) -> Result<(), ScanErr>;
    async fn update_rules(
        &self,
        deployment: Deployment,
        rules: Vec<UploadRule>,
    ) -> Result<(), ScanErr>;
    async fn scan(&self) -> Result<(), ScanErr>;
    async fn subscribe(&self) -> Result<broadcast::Receiver<ScanEvent>, ScanErr>;
    async fn shutdown(&self) -> Result<(), ScanErr>;
    async fn prune(&self, before: DateTime<Utc>) -> Result<(), ScanErr>;
}

pub enum Command {
    ClearRules {
        respond_to: oneshot::Sender<Result<(), ScanErr>>,
    },
    UpdateRules {
        deployment: Box<Deployment>,
        rules: Vec<UploadRule>,
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
    #[cfg(feature = "test")]
    GetLedgerCount {
        respond_to: oneshot::Sender<Result<usize, ScanErr>>,
    },
    Prune {
        before: DateTime<Utc>,
        respond_to: oneshot::Sender<Result<(), ScanErr>>,
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
                Command::ClearRules { respond_to } => {
                    dispatch!(
                        self.scanner.clear_rules().await,
                        respond_to,
                        "Actor failed to send clear rules response"
                    );
                }
                Command::UpdateRules {
                    deployment,
                    rules,
                    respond_to,
                } => {
                    dispatch!(
                        self.scanner.update_rules(*deployment, rules).await,
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
                    dispatch!(
                        self.scanner.get_rules().await,
                        respond_to,
                        "Actor failed to send get rules response"
                    );
                }
                #[cfg(feature = "test")]
                Command::GetLedgerCount { respond_to } => {
                    dispatch!(
                        self.scanner.get_ledger_count().await,
                        respond_to,
                        "Actor failed to send get ledger count response"
                    );
                }
                Command::Prune { before, respond_to } => {
                    dispatch!(
                        self.scanner.prune(before).await,
                        respond_to,
                        "Actor failed to send prune response"
                    );
                }
            }
        }
    }
}

/// Command handle to the [`SingleThreadScanner`] actor. Reactive, not
/// self-scheduling: each [`scan`](ScannerExt::scan) call performs exactly one
/// discover/evaluate pass and reports newly-stable files exactly once. The
/// cadence that drives repeated passes is imposed by an external driver (future
/// PR), not by this type.
#[derive(Debug)]
pub struct Scanner {
    sender: mpsc::Sender<Command>,
}

impl Scanner {
    pub fn spawn(buffer_size: usize, args: ScannerArgs) -> Result<(Self, JoinHandle<()>), ScanErr> {
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
    async fn clear_rules(&self) -> Result<(), ScanErr> {
        self.send_command(|tx| Command::ClearRules { respond_to: tx })
            .await?
    }

    async fn update_rules(
        &self,
        deployment: Deployment,
        rules: Vec<UploadRule>,
    ) -> Result<(), ScanErr> {
        self.send_command(|tx| Command::UpdateRules {
            deployment: Box::new(deployment),
            rules,
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

    async fn prune(&self, before: DateTime<Utc>) -> Result<(), ScanErr> {
        self.send_command(|tx| Command::Prune {
            before,
            respond_to: tx,
        })
        .await?
    }
}

#[cfg(test)]
mod tests {
    // standard crates
    use std::collections::BTreeSet;
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::sync::Arc;

    // internal crates
    use super::{ScanEvent, ScannerExt, DEFAULT_BROADCAST_CAPACITY};
    use super::{Scanner, ScannerArgs, SingleThreadScanner, StableFile, Worker};
    use crate::filesys::{self, Dir, PathExt};
    use crate::models::{Deployment, DplActivity, UploadRule, UploadRuleSource};

    // standard crates
    use std::path::PathBuf;

    // external crates
    use chrono::{DateTime, Utc};
    use tokio::sync::mpsc;

    /// A controllable monotonic-ish clock for deterministic worker tests. Holds the
    /// current time as epoch seconds in a shared atomic so a test can step it
    /// forward independently of wall-clock time. `now_fn()` produces the
    /// `Fn() -> DateTime<Utc>` closure that the worker's injected `now_fn` expects.
    #[derive(Clone)]
    pub struct Clock {
        secs: Arc<AtomicI64>,
    }

    impl Clock {
        pub fn new(start_secs: i64) -> Self {
            Self {
                secs: Arc::new(AtomicI64::new(start_secs)),
            }
        }

        pub fn now_fn(&self) -> impl Fn() -> DateTime<Utc> {
            let secs = self.secs.clone();
            move || DateTime::from_timestamp(secs.load(Ordering::SeqCst), 0).unwrap()
        }

        pub fn advance(&self, secs: i64) {
            self.secs.fetch_add(secs, Ordering::SeqCst);
        }
    }

    // =============================== TEST HELPERS ================================= //

    /// A Deployed deployment with the given id (release_id mirrors the id).
    fn deployment(id: &str) -> Deployment {
        Deployment {
            id: id.to_string(),
            activity_status: DplActivity::Deployed,
            release_id: id.to_string(),
            ..Default::default()
        }
    }

    /// Build an UploadRule with a pinned upload_collection_id (plus id/glob/window).
    fn rule_in_collection(
        id: &str,
        collection_id: &str,
        glob: &str,
        stability_window_secs: i32,
    ) -> UploadRule {
        UploadRule {
            id: id.to_string(),
            upload_collection_id: collection_id.to_string(),
            source: UploadRuleSource {
                glob: glob.to_string(),
                stability_window_secs,
            },
            ..Default::default()
        }
    }

    /// Spawn a scanner actor with a deterministic injected clock and the default
    /// broadcast capacity.
    fn spawn_scanner(clock: &Clock) -> Scanner {
        spawn_scanner_with_capacity(clock, DEFAULT_BROADCAST_CAPACITY)
    }

    /// Spawn a scanner with a deterministic injected clock and an explicit broadcast
    /// capacity.
    fn spawn_scanner_with_capacity(clock: &Clock, capacity: usize) -> Scanner {
        let (scanner, _h) = Scanner::spawn(
            64,
            ScannerArgs {
                now_fn: Arc::new(clock.now_fn()),
                broadcast_capacity: capacity,
            },
        )
        .unwrap();
        scanner
    }

    /// temp dir + `*.mcap` glob + spawned scanner + one deployed "coll" rule
    /// (deployment "d", rule "r", window `window`). Returns
    /// (dir, base_path, clock, scanner). Hold `dir` to keep the temp tree alive.
    async fn single_coll(window: i32) -> (Dir, PathBuf, Clock, Scanner) {
        let dir = filesys::dirs::create_temp("testing").await.unwrap();
        let base = dir.path().clone();
        let glob = format!("{}/*.mcap", base.display());
        let clock = Clock::new(1000);
        let scanner = spawn_scanner(&clock);
        scanner
            .update_rules(
                deployment("d"),
                vec![rule_in_collection("r", "coll", &glob, window)],
            )
            .await
            .unwrap();
        (dir, base, clock, scanner)
    }

    /// The set of rule ids currently held by the scanner.
    fn rule_ids(rules: &[UploadRule]) -> BTreeSet<String> {
        rules.iter().map(|r| r.id.clone()).collect()
    }

    /// The set of collection ids currently held by the scanner.
    fn collection_ids(rules: &[UploadRule]) -> BTreeSet<String> {
        rules
            .iter()
            .map(|r| r.upload_collection_id.clone())
            .collect()
    }

    /// Write `bytes` to `dir/name` and return the absolute path.
    fn write(dir: &std::path::Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    /// The file name of a StableFile's underlying File.
    fn stable_name(sf: &StableFile) -> String {
        sf.file
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string()
    }

    // ============================ B1: PREEXISTING FILES =========================== //

    // A file that already matches the glob BEFORE the collection is created is
    // snapshotted as preexisting and is never reported, even past the window.
    #[tokio::test]
    async fn preexisting_file_is_ignored() {
        let dir = filesys::dirs::create_temp("testing").await.unwrap();
        let base = dir.path().clone();
        write(&base, "pre.mcap", b"aaa");
        let glob = format!("{}/*.mcap", base.display());

        let clock = Clock::new(1000);
        let scanner = spawn_scanner(&clock);
        scanner
            .update_rules(
                deployment("d"),
                vec![rule_in_collection("r", "coll", &glob, 0)],
            )
            .await
            .unwrap();

        // discover pass, then advance well past the window and evaluate: nothing.
        scanner.scan().await.unwrap();
        clock.advance(100);
        scanner.scan().await.unwrap();
        scanner.scan().await.unwrap();
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 0);
    }

    // Once a preexisting file changes (size/mtime), the metadata comparison releases
    // it and it becomes a candidate that can go stable.
    #[tokio::test]
    async fn changed_preexisting_file_is_promoted() {
        let dir = filesys::dirs::create_temp("testing").await.unwrap();
        let base = dir.path().clone();
        let path = write(&base, "pre.mcap", b"aaa");
        let glob = format!("{}/*.mcap", base.display());

        let clock = Clock::new(1000);
        let scanner = spawn_scanner(&clock);
        scanner
            .update_rules(
                deployment("d"),
                vec![rule_in_collection("r", "coll", &glob, 0)],
            )
            .await
            .unwrap();

        // rewrite with a different size so metadata differs from the preexisting snapshot.
        std::fs::write(&path, b"bbbbbbbb").unwrap();

        // discover (now a candidate), then evaluate past the window.
        clock.advance(1);
        scanner.scan().await.unwrap();
        clock.advance(1);
        scanner.scan().await.unwrap();
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 1);
    }

    // update_rules re-snapshots preexisting: pushing the same collection again after a
    // new file appears suppresses that file (it is re-discovered as preexisting).
    #[tokio::test]
    async fn update_rules_resnapshots_preexisting() {
        let (_dir, base, clock, scanner) = single_coll(0).await;
        let glob = format!("{}/*.mcap", base.display());

        // a file appears after the collection was created.
        write(&base, "late.mcap", b"aaa");

        // push the SAME collection again: update_config re-runs discover_preexisting,
        // so the now-present file is snapshotted as preexisting.
        scanner
            .update_rules(
                deployment("d"),
                vec![rule_in_collection("r2", "coll", &glob, 0)],
            )
            .await
            .unwrap();

        scanner.scan().await.unwrap();
        clock.advance(100);
        scanner.scan().await.unwrap();
        scanner.scan().await.unwrap();
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 0);
    }

    // ================== B2: DISCOVERY (deployed) vs EVALUATION (all) ============== //

    // A deployed collection discovers a new (non-preexisting) file and later evaluates
    // it to stable.
    #[tokio::test]
    async fn deployed_collection_discovers_and_evaluates() {
        let (_dir, base, clock, scanner) = single_coll(0).await;

        // file created after the collection => not preexisting => a candidate.
        write(&base, "new.mcap", b"aaa");

        scanner.scan().await.unwrap();
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 0);
        clock.advance(1);
        scanner.scan().await.unwrap();
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 1);
    }

    // A non-deployed scanner keeps evaluating its existing candidate pool but does NOT
    // discover new files. clear_rules drops the collection from `deployed` but leaves
    // the scanner in place.
    #[tokio::test]
    async fn inactive_scanner_evaluates_but_does_not_discover() {
        let (_dir, base, clock, scanner) = single_coll(10).await;

        // discover one candidate while deployed.
        write(&base, "first.mcap", b"aaa");
        scanner.scan().await.unwrap();

        // no longer deployed: keeps the scanner but stops discovering.
        scanner.clear_rules().await.unwrap();

        // a new file added now must NOT be discovered.
        write(&base, "second.mcap", b"bbb");

        // subscribe before the evaluating scan so we observe the emitted StableFile.
        let mut rx = scanner.subscribe().await.unwrap();

        // advance past the window and evaluate: only the pre-existing candidate goes
        // stable; the post-clear file was never discovered.
        clock.advance(10);
        scanner.scan().await.unwrap();

        // Under #115's report-once model, promoting the last candidate empties the
        // inactive scanner's pool, so it is pruned in this same scan() tick and its
        // ledger is dropped. The observable outcome is the emitted StableFile, not a
        // surviving ledger. Exactly one event fires, for first.mcap (proving the
        // post-clear second.mcap was never discovered).
        let ScanEvent::StableFile(sf) = rx.recv().await.unwrap();
        assert_eq!(stable_name(&sf), "first.mcap".to_string());
        assert!(
            rx.try_recv().is_err(),
            "expected exactly one StableFile event"
        );

        // the now-empty inactive scanner was pruned this tick.
        assert!(!collection_ids(&scanner.get_rules().await.unwrap()).contains("coll"));
    }

    // An inactive scanner with no remaining candidates is pruned from the map on the
    // next scan (get_rules no longer reflects it).
    #[tokio::test]
    async fn inactive_empty_scanner_is_pruned() {
        let (_dir, _base, _clock, scanner) = single_coll(0).await;
        // no candidates were ever discovered (empty glob dir).
        assert_eq!(collection_ids(&scanner.get_rules().await.unwrap()).len(), 1);

        scanner.clear_rules().await.unwrap();
        // first scan finds no candidates for the now-inactive scanner => prune it.
        scanner.scan().await.unwrap();
        assert!(scanner.get_rules().await.unwrap().is_empty());
    }

    // ============================= B3: STABILITY WINDOW =========================== //

    // Unchanged for exactly the window (>= boundary) => stable.
    #[tokio::test]
    async fn window_boundary_is_stable() {
        let (_dir, base, clock, scanner) = single_coll(10).await;
        write(&base, "z.mcap", b"zzz");

        // discover at t=1000 (first observation).
        scanner.scan().await.unwrap();
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 0);
        // exactly window seconds later => stable.
        clock.advance(10);
        scanner.scan().await.unwrap();
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 1);
    }

    // Not-yet-elapsed (window - 1) => not stable.
    #[tokio::test]
    async fn window_not_yet_elapsed() {
        let (_dir, base, clock, scanner) = single_coll(10).await;
        write(&base, "y.mcap", b"yyy");

        scanner.scan().await.unwrap();
        clock.advance(9);
        scanner.scan().await.unwrap();
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 0);
    }

    // A file changed after discovery is dropped as a candidate (its metadata no longer
    // matches the discovery observation) and does not go stable on that pass.
    #[tokio::test]
    async fn change_after_discovery_drops_candidate() {
        let dir = filesys::dirs::create_temp("testing").await.unwrap();
        let base = dir.path().clone();
        let glob = format!("{}/*.mcap", base.display());
        let path = write(&base, "chg.mcap", b"aaa");

        let clock = Clock::new(1000);
        let scanner = spawn_scanner(&clock);
        scanner
            .update_rules(
                deployment("d"),
                vec![rule_in_collection("r", "coll", &glob, 10)],
            )
            .await
            .unwrap();
        // the file above is preexisting; make it a candidate by changing size then
        // advancing so a discover pass picks it up.
        std::fs::write(&path, b"bbbbbbbb").unwrap();
        clock.advance(1);
        scanner.scan().await.unwrap(); // discover candidate at t=1001

        // change the file again before the window elapses: on evaluation the fresh
        // observation differs from the discovery observation => Unstable => dropped.
        std::fs::write(&path, b"cccccccccccc").unwrap();
        clock.advance(10);
        scanner.scan().await.unwrap(); // evaluate (dropped) + re-discover at new size
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 0);

        // the re-discovered candidate (stable size now) can still eventually go stable.
        clock.advance(10);
        scanner.scan().await.unwrap();
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 1);
    }

    // A candidate file deleted before evaluation hits the Unstable (deleted) branch
    // and is never reported.
    #[tokio::test]
    async fn deleted_before_stable_is_dropped() {
        let (_dir, base, clock, scanner) = single_coll(10).await;
        let path = write(&base, "gone.mcap", b"aaa");

        scanner.scan().await.unwrap(); // discover candidate at t=1000
        std::fs::remove_file(&path).unwrap();
        clock.advance(10);
        scanner.scan().await.unwrap(); // window elapsed but file is gone => Unstable
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 0);
    }

    // window=0 still needs a prior observation: one discover pass then a later evaluate
    // pass to become stable.
    #[tokio::test]
    async fn window_zero_needs_prior_observation() {
        let (_dir, base, clock, scanner) = single_coll(0).await;
        write(&base, "w.mcap", b"aaa");

        scanner.scan().await.unwrap(); // discover only
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 0);
        clock.advance(1);
        scanner.scan().await.unwrap(); // evaluate => stable
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 1);
    }

    // ==================== B4: DEDUP LEDGER + PRUNE + get_ledger_count ============= //

    // A stable file is reported exactly once across repeated scans.
    #[tokio::test]
    async fn dedup_reports_once() {
        let (_dir, base, clock, scanner) = single_coll(0).await;
        write(&base, "once.mcap", b"aaa");

        scanner.scan().await.unwrap();
        for _ in 0..4 {
            clock.advance(1);
            scanner.scan().await.unwrap();
        }
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 1);
    }

    // prune(before) with `before` AFTER the recorded first_observed_at drops the
    // ledger entry; with `before` earlier retains it.
    #[tokio::test]
    async fn prune_drops_and_retains() {
        let (_dir, base, clock, scanner) = single_coll(0).await;
        write(&base, "p.mcap", b"aaa");

        // discover at t=1000, evaluate stable at t=1001. first_observed_at == 1000.
        scanner.scan().await.unwrap();
        clock.advance(1);
        scanner.scan().await.unwrap();
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 1);

        // retain: before (t=999) <= first_observed_at (1000) => keep.
        scanner
            .prune(chrono::DateTime::from_timestamp(999, 0).unwrap())
            .await
            .unwrap();
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 1);

        // drop: before (t=1001) > first_observed_at (1000) => remove.
        scanner
            .prune(chrono::DateTime::from_timestamp(1001, 0).unwrap())
            .await
            .unwrap();
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 0);
    }

    // Two distinct collections matching the same file do NOT share dedup state, so the
    // summed ledger count is 2 (per-collection isolation).
    #[tokio::test]
    async fn distinct_collections_do_not_share_dedup() {
        let dir = filesys::dirs::create_temp("testing").await.unwrap();
        let base = dir.path().clone();
        let glob = format!("{}/*.mcap", base.display());

        let clock = Clock::new(1000);
        let scanner = spawn_scanner(&clock);
        scanner
            .update_rules(
                deployment("d"),
                vec![
                    rule_in_collection("c1", "coll-1", &glob, 0),
                    rule_in_collection("c2", "coll-2", &glob, 0),
                ],
            )
            .await
            .unwrap();
        write(&base, "shared.mcap", b"sss");

        scanner.scan().await.unwrap();
        clock.advance(1);
        scanner.scan().await.unwrap();
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 2);
    }

    // ================================ B5: update_rules =========================== //

    // Pushing a new collection id creates a scanner reflected in get_rules.
    #[tokio::test]
    async fn update_rules_creates_collection() {
        let clock = Clock::new(1000);
        let scanner = spawn_scanner(&clock);
        scanner
            .update_rules(
                deployment("d"),
                vec![rule_in_collection("r", "coll", "/none/*.mcap", 0)],
            )
            .await
            .unwrap();
        assert_eq!(
            collection_ids(&scanner.get_rules().await.unwrap()),
            BTreeSet::from(["coll".to_string()])
        );
        assert_eq!(
            rule_ids(&scanner.get_rules().await.unwrap()),
            BTreeSet::from(["r".to_string()])
        );
    }

    // Pushing the SAME collection id with a new rule updates config in place and
    // carries over ledger state, so an already-reported file is not re-reported.
    #[tokio::test]
    async fn update_rules_updates_in_place_carrying_state() {
        let dir = filesys::dirs::create_temp("testing").await.unwrap();
        let base = dir.path().clone();
        let glob = format!("{}/*.mcap", base.display());

        let clock = Clock::new(1000);
        let scanner = spawn_scanner(&clock);

        let mut v1 = rule_in_collection("r1", "coll", &glob, 0);
        v1.digest = "d1".to_string();
        scanner
            .update_rules(deployment("d"), vec![v1])
            .await
            .unwrap();

        // file appears after creation and goes stable.
        write(&base, "carry.mcap", b"ccc");
        scanner.scan().await.unwrap();
        clock.advance(1);
        scanner.scan().await.unwrap();
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 1);

        // v2: SAME collection, new rule id + digest.
        let mut v2 = rule_in_collection("r2", "coll", &glob, 0);
        v2.digest = "d2".to_string();
        scanner
            .update_rules(deployment("d"), vec![v2])
            .await
            .unwrap();

        // the swap carried the dedup state: no re-report.
        clock.advance(1);
        scanner.scan().await.unwrap();
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 1);

        let rules = scanner.get_rules().await.unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].digest, "d2".to_string());
    }

    // A rule set with two rules sharing one collection id is rejected BEFORE any state
    // mutation.
    #[tokio::test]
    async fn update_rules_duplicate_collection_id_errors_without_mutation() {
        let clock = Clock::new(1000);
        let scanner = spawn_scanner(&clock);

        // seed a known-good single collection first.
        scanner
            .update_rules(
                deployment("d"),
                vec![rule_in_collection("r0", "coll-existing", "/none/*.mcap", 0)],
            )
            .await
            .unwrap();
        let before = collection_ids(&scanner.get_rules().await.unwrap());

        // push a set with a duplicate collection id => error.
        let err = scanner
            .update_rules(
                deployment("d"),
                vec![
                    rule_in_collection("a", "dup", "/none/*.mcap", 0),
                    rule_in_collection("b", "dup", "/none/*.mcap", 0),
                ],
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            crate::scan::ScanErr::DuplicateCollectionID(_)
        ));

        // no state mutated: the pre-existing collection is untouched, the dup set was
        // not applied.
        assert_eq!(collection_ids(&scanner.get_rules().await.unwrap()), before);
    }

    // The deployed set is replaced on each update_rules: after [A,B] then [C], only C
    // discovers; A and B drain-and-prune on subsequent scans.
    #[tokio::test]
    async fn update_rules_replaces_deployed_set() {
        let clock = Clock::new(1000);
        let scanner = spawn_scanner(&clock);

        scanner
            .update_rules(
                deployment("d"),
                vec![
                    rule_in_collection("a", "coll-A", "/none/*.mcap", 0),
                    rule_in_collection("b", "coll-B", "/none/*.mcap", 0),
                ],
            )
            .await
            .unwrap();

        scanner
            .update_rules(
                deployment("d"),
                vec![rule_in_collection("c", "coll-C", "/none/*.mcap", 0)],
            )
            .await
            .unwrap();

        // A and B are no longer deployed and have no candidates => pruned next scan.
        scanner.scan().await.unwrap();
        assert_eq!(
            collection_ids(&scanner.get_rules().await.unwrap()),
            BTreeSet::from(["coll-C".to_string()])
        );
    }

    // ================================ B6: clear_rules ============================ //

    // clear_rules empties `deployed` but leaves scanners in place; a subsequent scan
    // still evaluates their remaining candidates. Here the candidate goes stable while
    // inactive (clear does not immediately drop the scanner or its pool).
    #[tokio::test]
    async fn clear_rules_still_evaluates_remaining_candidates() {
        let (_dir, base, clock, scanner) = single_coll(0).await;
        write(&base, "drain.mcap", b"aaa");

        // discover a candidate while deployed.
        scanner.scan().await.unwrap();
        scanner.clear_rules().await.unwrap();

        // the scanner is still present (clear only emptied the deployed set).
        assert_eq!(collection_ids(&scanner.get_rules().await.unwrap()).len(), 1);

        // subscribe before the evaluating scan so we observe the emitted StableFile.
        let mut rx = scanner.subscribe().await.unwrap();

        // evaluate: the still-present candidate goes stable even though inactive.
        clock.advance(1);
        scanner.scan().await.unwrap();

        // Under #115's report-once model, promoting this last candidate empties the
        // inactive scanner's pool, so it is pruned in this same scan() tick and its
        // ledger is dropped. The observable outcome is the emitted StableFile, not a
        // surviving ledger.
        let ScanEvent::StableFile(sf) = rx.recv().await.unwrap();
        assert_eq!(stable_name(&sf), "drain.mcap".to_string());
        assert!(
            rx.try_recv().is_err(),
            "expected exactly one StableFile event"
        );

        // the now-empty inactive scanner was pruned this tick.
        assert!(scanner.get_rules().await.unwrap().is_empty());
    }

    // An inactive scanner whose candidate becomes Unstable (deleted) has no remaining
    // candidates and is pruned on the next scan (drain-then-prune).
    #[tokio::test]
    async fn clear_rules_drains_unstable_candidate_then_prunes() {
        let (_dir, base, clock, scanner) = single_coll(5).await;
        let path = write(&base, "drain.mcap", b"aaa");

        // discover a candidate while deployed.
        scanner.scan().await.unwrap();
        scanner.clear_rules().await.unwrap();
        assert_eq!(collection_ids(&scanner.get_rules().await.unwrap()).len(), 1);

        // delete the file so evaluation drops the candidate (Unstable), emptying the
        // inactive scanner's pool.
        std::fs::remove_file(&path).unwrap();
        clock.advance(5);
        scanner.scan().await.unwrap(); // evaluate => Unstable => candidate removed
                                       // the now-empty inactive scanner is pruned on this pass.
        assert!(scanner.get_rules().await.unwrap().is_empty());
    }

    // clear_rules on an empty scanner is a no-op (no panic).
    #[tokio::test]
    async fn clear_rules_on_empty_is_noop() {
        let clock = Clock::new(1000);
        let scanner = spawn_scanner(&clock);
        scanner.clear_rules().await.unwrap();
        scanner.scan().await.unwrap();
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 0);
    }

    // =========================== B7: STABLEFILE EMISSION ========================= //

    // subscribe() before scan(): a StableFile event carries the expected payload.
    #[tokio::test]
    async fn subscribe_receives_stable_file_payload() {
        let dir = filesys::dirs::create_temp("testing").await.unwrap();
        let base = dir.path().clone();
        let glob = format!("{}/*.mcap", base.display());

        let clock = Clock::new(1000);
        let scanner = spawn_scanner(&clock);
        scanner
            .update_rules(
                deployment("dpl-1"),
                vec![rule_in_collection("rule-1", "coll", &glob, 0)],
            )
            .await
            .unwrap();
        write(&base, "emit.mcap", b"aaaa");

        let mut rx = scanner.subscribe().await.unwrap();

        scanner.scan().await.unwrap(); // discover
        clock.advance(1);
        scanner.scan().await.unwrap(); // evaluate => emit

        let ScanEvent::StableFile(sf) = rx.recv().await.unwrap();
        // lint:allow(field-by-field-assert)
        assert_eq!(stable_name(&sf), "emit.mcap".to_string());
        assert_eq!(sf.size, 4);
        assert_eq!(sf.deployment_id, "dpl-1".to_string());
        // NOTE: the src sets upload_rule_id from the collection id (observe_file).
        assert_eq!(sf.upload_rule_id, "coll".to_string());
        assert!(sf.first_observed_at <= sf.last_observed_at);
    }

    // A first-stable file (no prior ledger entry) carries the REAL file hash on its
    // emitted StableFile.digest, not an empty string. Regression guard for the
    // no-previous branch computing Some(files::hash(..)) rather than None/"".
    #[tokio::test]
    async fn first_stable_file_carries_real_digest() {
        let dir = filesys::dirs::create_temp("testing").await.unwrap();
        let base = dir.path().clone();
        let glob = format!("{}/*.mcap", base.display());

        let clock = Clock::new(1000);
        let scanner = spawn_scanner(&clock);
        scanner
            .update_rules(
                deployment("dpl-1"),
                vec![rule_in_collection("rule-1", "coll", &glob, 0)],
            )
            .await
            .unwrap();
        write(&base, "digest.mcap", b"aaaa");

        let mut rx = scanner.subscribe().await.unwrap();

        scanner.scan().await.unwrap(); // discover
        clock.advance(1);
        scanner.scan().await.unwrap(); // evaluate => emit

        let ScanEvent::StableFile(sf) = rx.recv().await.unwrap();
        // The digest must be the real SHA-256 of b"aaaa", never empty.
        assert!(
            !sf.digest.is_empty(),
            "first-stable digest must not be empty"
        );
        assert_eq!(
            sf.digest,
            "sha256:61be55a8e2f6b4e172338bddf184d6dbee29c98853e0a0485ecee7f27b9af0b4".to_string(),
        );
    }

    // scan() producing stable files with NO subscriber does not error (debug branch).
    #[tokio::test]
    async fn emit_with_no_subscriber_does_not_error() {
        let (_dir, base, clock, scanner) = single_coll(0).await;
        write(&base, "nosub.mcap", b"aaa");

        scanner.scan().await.unwrap();
        clock.advance(1);
        scanner.scan().await.unwrap(); // emits with no subscriber, must not error
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 1);
    }

    // A subscriber that does not drain observes RecvError::Lagged once more events than
    // the broadcast capacity are produced.
    #[tokio::test]
    async fn tiny_capacity_subscriber_lags() {
        let dir = filesys::dirs::create_temp("testing").await.unwrap();
        let base = dir.path().clone();
        let glob = format!("{}/*.mcap", base.display());

        let clock = Clock::new(1000);
        let scanner = spawn_scanner_with_capacity(&clock, 1);
        scanner
            .update_rules(
                deployment("d"),
                vec![rule_in_collection("r", "coll", &glob, 0)],
            )
            .await
            .unwrap();

        let mut rx = scanner.subscribe().await.unwrap();

        // produce several stable files without draining rx (capacity is 1).
        for i in 0..4 {
            write(&base, &format!("f{i}.mcap"), b"aaa");
            scanner.scan().await.unwrap(); // discover fN
            clock.advance(1);
            scanner.scan().await.unwrap(); // evaluate => emit fN
        }

        // the undrained receiver has lagged behind the capacity.
        let mut lagged = false;
        for _ in 0..8 {
            match rx.try_recv() {
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {
                    lagged = true;
                    break;
                }
                Ok(_) => continue,
                Err(_) => break,
            }
        }
        assert!(lagged, "expected the undrained subscriber to lag");
    }

    // ============================= B8: ACTOR LIFECYCLE =========================== //

    // shutdown stops the actor loop; the task completes and later commands error.
    #[tokio::test]
    async fn shutdown_stops_actor_and_later_commands_error() {
        let clock = Clock::new(1000);
        let (scanner, handle) = Scanner::spawn(
            64,
            ScannerArgs {
                now_fn: Arc::new(clock.now_fn()),
                ..ScannerArgs::default()
            },
        )
        .unwrap();

        scanner.shutdown().await.unwrap();
        handle.await.unwrap();

        let err = scanner.scan().await.unwrap_err();
        assert!(matches!(err, crate::scan::ScanErr::SendActorMessageErr(_)));
    }

    // With no rules the scan is a repeated no-op.
    #[tokio::test]
    async fn empty_set_scan_is_noop() {
        let clock = Clock::new(1000);
        let scanner = spawn_scanner(&clock);
        for _ in 0..5 {
            scanner.scan().await.unwrap();
        }
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 0);
    }

    // ===================== B9: MANUAL ACTOR CONSTRUCTION ========================= //

    // Build the actor by hand rather than via `Scanner::spawn`: wire the channel,
    // construct `SingleThreadScanner` + `Worker::new`, spawn `worker.run()`, and wrap
    // the sender with `Scanner::new`. Covers `Worker::new` (only otherwise reached via
    // spawn) and the dead `Scanner::new(sender)` (no production caller). A single
    // get_ledger_count/shutdown round-trip proves the hand-built actor works.
    #[tokio::test]
    async fn worker_new_and_scanner_new_round_trip() {
        let (tx, rx) = mpsc::channel(64);
        let single = SingleThreadScanner::new(ScannerArgs::default());
        let worker = Worker::new(single, rx);
        let handle = tokio::spawn(worker.run());

        let scanner = Scanner::new(tx);

        // the hand-built actor answers commands: empty ledger.
        assert_eq!(scanner.get_ledger_count().await.unwrap(), 0);

        // and shuts down cleanly; later commands then error on a closed channel.
        scanner.shutdown().await.unwrap();
        handle.await.unwrap();
        let err = scanner.get_ledger_count().await.unwrap_err();
        assert!(matches!(err, crate::scan::ScanErr::SendActorMessageErr(_)));
    }
}
