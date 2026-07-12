// standard crates
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

// internal crates
use crate::models::{Deployment, UploadCollectionID, UploadRule};
pub use crate::scan::state::{Config, StableFile};
use crate::scan::{
    collection::CollectionScanner,
    errors::*,
    state::{PersistedState, ScanStateFile, State},
};
use crate::trace;

// external crates
use chrono::{DateTime, Utc};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

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
    pub state_file: Option<ScanStateFile>,
}

impl Default for ScannerArgs {
    fn default() -> Self {
        Self {
            now_fn: Arc::new(Utc::now),
            broadcast_capacity: DEFAULT_BROADCAST_CAPACITY,
            state_file: None,
        }
    }
}

pub struct SingleThreadScanner {
    scanners: HashMap<UploadCollectionID, CollectionScanner>,
    deployed: HashSet<UploadCollectionID>,
    now_fn: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
    subscriber_tx: broadcast::Sender<ScanEvent>,
    state_file: Option<ScanStateFile>,
    // Persisted collection states waiting for their collection id to be
    // deployed again; consumed by `update_rules` via `CollectionScanner::restore`.
    restored: HashMap<UploadCollectionID, State>,
}

impl SingleThreadScanner {
    pub fn new(args: ScannerArgs) -> Self {
        let (subscriber_tx, _) = broadcast::channel(args.broadcast_capacity);
        let restored = args
            .state_file
            .as_ref()
            .map(|state_file| state_file.read().collections.clone())
            .unwrap_or_default();
        Self {
            scanners: HashMap::new(),
            deployed: HashSet::new(),
            now_fn: args.now_fn,
            subscriber_tx,
            state_file: args.state_file,
            restored,
        }
    }

    /// Persist a snapshot of every collection's state (live and restored) to
    /// the state file. Failures are logged and swallowed: degraded dedup
    /// beats no uploads.
    async fn persist(&mut self) {
        let Some(state_file) = self.state_file.as_mut() else {
            return;
        };
        let mut collections: HashMap<UploadCollectionID, State> = self
            .scanners
            .iter()
            .map(|(cid, scanner)| (cid.clone(), scanner.state().clone()))
            .collect();
        collections.extend(
            self.restored
                .iter()
                .map(|(cid, state)| (cid.clone(), state.clone())),
        );
        if let Err(err) = state_file.patch(PersistedState { collections }).await {
            warn!("scan: failed to persist scanner state: {err}");
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
                    let scanner = match self.restored.remove(&rule.upload_collection_id) {
                        Some(state) => CollectionScanner::restore(state, config, now).await?,
                        None => CollectionScanner::new(config, now).await?,
                    };
                    self.scanners
                        .insert(rule.upload_collection_id.clone(), scanner);
                }
            }
        }

        self.deployed = deployed;

        self.persist().await;

        Ok(())
    }

    async fn scan(&mut self) -> Result<(), ScanErr> {
        let mut stable_files = Vec::new();
        let mut inactive_colls = Vec::new();

        let now = (self.now_fn)();

        // evaludate candidates for all scanners
        for (cid, scanner) in self.scanners.iter_mut() {
            match scanner.evaluate_candidates(now).await {
                Ok(stable) => stable_files.extend(stable),
                Err(err) => warn!("scan: evaluate failed for collection {cid}: {err}"),
            }

            // discover candidates for deployed scanners
            if self.deployed.contains(cid) {
                if let Err(err) = scanner.discover_candidates(now).await {
                    warn!("scan: discover failed for collection {cid}: {err}");
                }
            // if the scanner has no candidates, it is inactive
            } else if !scanner.has_candidates() {
                inactive_colls.push(cid.clone());
            }
        }

        // prune inactive collection scanners
        for cid in inactive_colls {
            self.scanners.remove(&cid);
        }

        self.emit_stable_files(stable_files);

        self.persist().await;

        Ok(())
    }

    async fn prune(&mut self, before: DateTime<Utc>) -> Result<(), ScanErr> {
        for scanner in self.scanners.values_mut() {
            scanner.prune_ledger(before)?;
        }
        for state in self.restored.values_mut() {
            state.prune_ledger(before)?;
        }
        // a restored entry with nothing left in its ledger and no in-flight
        // candidates holds nothing worth restoring
        self.restored
            .retain(|_, state| state.ledger_count() > 0 || state.has_candidates());
        self.persist().await;
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
                    // land the final state so a graceful shutdown never loses it
                    self.scanner.persist().await;
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
    use crate::filesys::{dirs, files, Dir, File, PathExt, WriteOptions};
    use crate::models::{Deployment, DplActivity, UploadRule, UploadRuleSource};

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

    /// Default `UploadRule::upload_collection_id` for `single_coll` and tests that
    /// redeploy the same scanner map entry.
    const DEFAULT_COLL_ID: &str = "coll";

    /// A Deployed deployment with the given id (release_id mirrors the id).
    fn deployment(id: &str) -> Deployment {
        Deployment {
            id: id.to_string(),
            activity_status: DplActivity::Deployed,
            release_id: id.to_string(),
            ..Default::default()
        }
    }

    /// Build an UploadRule with a pinned `upload_collection_id` (plus rule id/glob/window).
    fn rule_in_collection(
        rule_id: &str,
        upload_collection_id: &str,
        glob: &str,
        stability_window_secs: i32,
    ) -> UploadRule {
        UploadRule {
            id: rule_id.to_string(),
            upload_collection_id: upload_collection_id.to_string(),
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
                state_file: None,
            },
        )
        .unwrap();
        scanner
    }

    /// temp dir + `*.mcap` glob + spawned scanner + one deployed rule whose
    /// `upload_collection_id` is [`DEFAULT_UPLOAD_COLLECTION_ID`] (deployment "d",
    /// rule "r", window `window`). Returns (dir, clock, scanner).
    /// Hold `dir` to keep the temp tree alive.
    async fn single_coll(window: i32) -> (dirs::TempDir, Clock, Scanner) {
        let dir = dirs::temp("testing").unwrap();
        let glob = format!("{}/*.mcap", dir.path().display());
        let clock = Clock::new(1000);
        let scanner = spawn_scanner(&clock);
        scanner
            .update_rules(
                deployment("d"),
                vec![rule_in_collection("r", DEFAULT_COLL_ID, &glob, window)],
            )
            .await
            .unwrap();
        (dir, clock, scanner)
    }

    /// The set of rule ids currently held by the scanner.
    fn rule_ids(rules: &[UploadRule]) -> BTreeSet<String> {
        rules.iter().map(|r| r.id.clone()).collect()
    }

    /// The set of `upload_collection_id` values currently held by the scanner.
    fn collection_ids(rules: &[UploadRule]) -> BTreeSet<String> {
        rules
            .iter()
            .map(|r| r.upload_collection_id.clone())
            .collect()
    }

    /// Write `bytes` to `dir/name` and return the file.
    async fn write(dir: &Dir, name: &str, bytes: &[u8]) -> File {
        let file = dir.file(name);
        files::write_bytes(&file, bytes, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();
        file
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

    /// Deploy `rules` under the fixed deployment "d". The rule vec stays explicit
    /// at the call site; only the boilerplate deployment + `.await.unwrap()` hides.
    async fn deploy(scanner: &Scanner, rules: Vec<UploadRule>) {
        scanner.update_rules(deployment("d"), rules).await.unwrap();
    }

    /// Advance the clock by `secs` and run one scan tick.
    async fn tick(scanner: &Scanner, clock: &Clock, secs: i64) {
        clock.advance(secs);
        scanner.scan().await.unwrap();
    }

    /// Run one scan tick.
    async fn scan_once(scanner: &Scanner) {
        scanner.scan().await.unwrap();
    }

    /// Subscribe to the scanner's event stream.
    async fn subscribe(scanner: &Scanner) -> tokio::sync::broadcast::Receiver<ScanEvent> {
        scanner.subscribe().await.unwrap()
    }

    /// The number of entries in the scanner's ledger.
    async fn ledger_count(scanner: &Scanner) -> usize {
        scanner.get_ledger_count().await.unwrap()
    }

    /// Collection ids with live scanner state, including inactive legacy scanners
    /// that are still draining candidates.
    async fn active_collections(scanner: &Scanner) -> BTreeSet<String> {
        collection_ids(&scanner.get_rules().await.unwrap())
    }

    /// Receive exactly one `StableFile` event, assert its file name is `name`, and
    /// assert no further event follows. The caller supplies the WHY exactly-one
    /// holds at its own site.
    async fn assert_one_stable(rx: &mut tokio::sync::broadcast::Receiver<ScanEvent>, name: &str) {
        let ScanEvent::StableFile(sf) = rx.recv().await.unwrap();
        assert_eq!(stable_name(&sf), name.to_string());
        assert!(
            rx.try_recv().is_err(),
            "expected exactly one StableFile event"
        );
    }

    mod construction {
        use super::*;

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

    mod subscribe {
        use super::*;

        // subscribe() before scan(): a StableFile event carries the expected payload.
        #[tokio::test]
        async fn subscribe_receives_stable_file_payload() {
            let dir = dirs::temp("testing").unwrap();
            let glob = format!("{}/*.mcap", dir.path().display());

            let clock = Clock::new(1000);
            let scanner = spawn_scanner(&clock);
            let rule = rule_in_collection("rule-1", DEFAULT_COLL_ID, &glob, 0);
            scanner
                .update_rules(deployment("dpl-1"), vec![rule])
                .await
                .unwrap();
            let file = write(&dir, "emit.mcap", b"aaaa").await;

            let digest = "sha256:61be55a8e2f6b4e172338bddf184d6dbee29c98853e0a0485ecee7f27b9af0b4"
                .to_string();
            let mtime = files::last_modified(&file).await.unwrap();
            let expected = StableFile {
                file: file.clone(),
                size: 4,
                digest,
                mtime: DateTime::<Utc>::from(mtime),
                mtime_aliases: vec![],
                first_observed_at: DateTime::from_timestamp(1000, 0).unwrap(),
                last_observed_at: DateTime::from_timestamp(1001, 0).unwrap(),
                deployment_id: "dpl-1".to_string(),
                upload_rule_id: DEFAULT_COLL_ID.to_string(),
            };

            let mut rx = subscribe(&scanner).await;

            scan_once(&scanner).await; // discover
            tick(&scanner, &clock, 1).await; // evaluate => emit

            let ScanEvent::StableFile(sf) = rx.recv().await.unwrap();
            assert_eq!(sf, expected);
        }

        // scan() producing stable files with NO subscriber does not error (debug branch).
        #[tokio::test]
        async fn emit_with_no_subscriber_does_not_error() {
            let (dir, clock, scanner) = single_coll(0).await;
            write(&dir, "nosub.mcap", b"aaa").await;

            scan_once(&scanner).await;
            tick(&scanner, &clock, 1).await; // emits with no subscriber, must not error
            assert_eq!(ledger_count(&scanner).await, 1);
        }
    }

    mod clear_rules {
        use super::*;

        #[tokio::test]
        async fn clear_rules_on_empty_is_noop() {
            let clock = Clock::new(1000);
            let scanner = spawn_scanner(&clock);
            scanner.clear_rules().await.unwrap();
            scan_once(&scanner).await;
            assert_eq!(ledger_count(&scanner).await, 0);
        }

        // clear_rules empties `deployed` but leaves scanners in place; a subsequent
        // scan still evaluates their remaining candidates. Here the candidate goes
        // stable while inactive (clear does not immediately drop the scanner or its
        // pool).
        #[tokio::test]
        async fn clear_rules_still_evaluates_remaining_candidates() {
            let (dir, clock, scanner) = single_coll(0).await;
            write(&dir, "drain.mcap", b"aaa").await;

            // discover a candidate while deployed.
            scan_once(&scanner).await;
            scanner.clear_rules().await.unwrap();

            // The collection remains active while its existing candidate drains.
            assert_eq!(active_collections(&scanner).await.len(), 1);

            // subscribe before the evaluating scan so we observe the emitted StableFile.
            let mut rx = subscribe(&scanner).await;
            tick(&scanner, &clock, 1).await;
            assert_one_stable(&mut rx, "drain.mcap").await;

            // the now-empty inactive scanner was pruned this tick.
            assert!(scanner.get_rules().await.unwrap().is_empty());
        }

        // An inactive scanner whose candidate becomes Unstable (deleted) has no remaining
        // candidates and is pruned on the next scan (drain-then-prune).
        #[tokio::test]
        async fn clear_rules_drains_unstable_candidate_then_prunes() {
            let (dir, clock, scanner) = single_coll(5).await;
            let file = write(&dir, "drain.mcap", b"aaa").await;

            // discover a candidate while deployed.
            scan_once(&scanner).await;
            scanner.clear_rules().await.unwrap();
            assert_eq!(active_collections(&scanner).await.len(), 1);

            // delete the file so evaluation drops the candidate (Unstable), emptying the
            // inactive scanner's pool.
            files::delete(&file).await.unwrap();
            clock.advance(5);
            scan_once(&scanner).await;
            assert!(scanner.get_rules().await.unwrap().is_empty());
        }
    }

    mod update_rules {
        use super::*;

        // A rule set with two rules sharing one collection id is rejected BEFORE any
        // state mutation.
        #[tokio::test]
        async fn update_rules_duplicate_collection_id_errors() {
            let clock = Clock::new(1000);
            let scanner = spawn_scanner(&clock);

            // seed a known-good single collection first.
            let rule0 = rule_in_collection("r0", "coll-existing", "/none/*.mcap", 0);
            deploy(&scanner, vec![rule0]).await;
            let before = active_collections(&scanner).await;

            // push a set with a duplicate collection id => error.
            let rulea = rule_in_collection("a", "dup", "/none/*.mcap", 0);
            let ruleb = rule_in_collection("b", "dup", "/none/*.mcap", 0);
            let rules = vec![rulea, ruleb];
            let err = scanner
                .update_rules(deployment("d"), rules)
                .await
                .unwrap_err();
            assert!(matches!(
                err,
                crate::scan::ScanErr::DuplicateCollectionID(_)
            ));

            // No state mutated: the existing active collection is untouched and the
            // duplicate set was not applied.
            assert_eq!(active_collections(&scanner).await, before);
        }

        // Pushing a new collection id creates an active scanner reflected in get_rules.
        #[tokio::test]
        async fn update_rules_creates_collection() {
            let clock = Clock::new(1000);
            let scanner = spawn_scanner(&clock);
            let rule = rule_in_collection("r", DEFAULT_COLL_ID, "/none/*.mcap", 0);
            deploy(&scanner, vec![rule]).await;
            assert_eq!(
                active_collections(&scanner).await,
                BTreeSet::from([DEFAULT_COLL_ID.to_string()])
            );
            assert_eq!(
                rule_ids(&scanner.get_rules().await.unwrap()),
                BTreeSet::from(["r".to_string()])
            );
        }

        // Pushing the SAME upload_collection_id with a new rule updates config in place
        // and carries over ledger state, so an already-reported file is not
        // re-reported.
        #[tokio::test]
        async fn update_rules_updates_in_place_carrying_state() {
            let dir = dirs::temp("testing").unwrap();
            let glob = format!("{}/*.mcap", dir.path().display());
            let upload_collection_id = DEFAULT_COLL_ID;

            let clock = Clock::new(1000);
            let scanner = spawn_scanner(&clock);

            let mut v1 = rule_in_collection("r1", upload_collection_id, &glob, 0);
            v1.digest = "d1".to_string();
            deploy(&scanner, vec![v1]).await;

            // file appears after creation and goes stable.
            write(&dir, "carry.mcap", b"ccc").await;
            scan_once(&scanner).await;
            tick(&scanner, &clock, 1).await;
            assert_eq!(ledger_count(&scanner).await, 1);

            // v2: same upload_collection_id, new rule id + digest.
            let mut v2 = rule_in_collection("r2", upload_collection_id, &glob, 0);
            v2.digest = "d2".to_string();
            deploy(&scanner, vec![v2]).await;

            // the swap carried the dedup state: no re-report.
            let mut rx = subscribe(&scanner).await;
            tick(&scanner, &clock, 1).await;
            assert_eq!(ledger_count(&scanner).await, 1);
            assert!(
                rx.try_recv().is_err(),
                "carried dedup state must not re-emit the already-reported file"
            );

            let rules = scanner.get_rules().await.unwrap();
            assert_eq!(rules.len(), 1);
            assert_eq!(rules[0].digest, "d2".to_string());
        }

        // The deployed set is replaced on each update_rules: after [A,B] then [C], only C
        // discovers; A and B drain-and-prune on subsequent scans.
        #[tokio::test]
        async fn update_rules_replaces_deployed_set() {
            let clock = Clock::new(1000);
            let scanner = spawn_scanner(&clock);

            let rulea = rule_in_collection("a", "coll-A", "/none/*.mcap", 0);
            let ruleb = rule_in_collection("b", "coll-B", "/none/*.mcap", 0);
            deploy(&scanner, vec![rulea, ruleb]).await;

            let rulec = rule_in_collection("c", "coll-C", "/none/*.mcap", 0);
            deploy(&scanner, vec![rulec]).await;

            // A and B are no longer deployed and have no candidates, so the next scan
            // removes them from the active scanner set.
            scan_once(&scanner).await;
            assert_eq!(
                active_collections(&scanner).await,
                BTreeSet::from(["coll-C".to_string()])
            );
        }

        // Replacing collection A with collection B keeps A's scanner alive long enough
        // to drain its existing candidates, while only B discovers newly added files.
        #[tokio::test]
        async fn update_rules_keeps_legacy_scanner_until_candidates_drain() {
            let dir = dirs::temp("testing").unwrap();
            let glob = format!("{}/*.mcap", dir.path().display());
            let clock = Clock::new(1000);
            let scanner = spawn_scanner(&clock);

            let rulelegacy = rule_in_collection("legacy-rule", "legacy", &glob, 10);
            deploy(&scanner, vec![rulelegacy]).await;
            write(&dir, "legacy.mcap", b"legacy").await;
            scan_once(&scanner).await;

            let rulecurrent = rule_in_collection("current-rule", "current", &glob, 10);
            deploy(&scanner, vec![rulecurrent]).await;
            // The deployed collection and draining legacy collection are both active.
            assert_eq!(
                active_collections(&scanner).await,
                BTreeSet::from(["current".to_string(), "legacy".to_string()])
            );

            write(&dir, "current.mcap", b"current").await;
            scan_once(&scanner).await;
            let mut rx = subscribe(&scanner).await;
            tick(&scanner, &clock, 10).await;

            let mut emitted = BTreeSet::new();
            while let Ok(ScanEvent::StableFile(stable)) = rx.try_recv() {
                emitted.insert((stable.upload_rule_id.clone(), stable_name(&stable)));
            }
            let event1 = ("current".to_string(), "current.mcap".to_string());
            let event2 = ("legacy".to_string(), "legacy.mcap".to_string());
            assert_eq!(emitted, BTreeSet::from([event1, event2]));
            // The legacy collection is no longer active once its candidate pool drains.
            assert_eq!(
                active_collections(&scanner).await,
                BTreeSet::from(["current".to_string()])
            );
        }

        // update_rules re-snapshots preexisting: redeploying the same
        // upload_collection_id after a new file appears suppresses that file (it is
        // re-discovered as preexisting).
        #[tokio::test]
        async fn update_rules_resnapshots_preexisting() {
            let (dir, clock, scanner) = single_coll(0).await;
            let glob = format!("{}/*.mcap", dir.path().display());

            // a file appears after the collection was created.
            write(&dir, "late.mcap", b"aaa").await;

            // same upload_collection_id as single_coll (rule id differs): update_config
            // re-runs discover_preexisting, so the now-present file is snapshotted as
            // preexisting.
            let rule = rule_in_collection("r2", DEFAULT_COLL_ID, &glob, 0);
            deploy(&scanner, vec![rule]).await;

            scan_once(&scanner).await;
            tick(&scanner, &clock, 100).await;
            scan_once(&scanner).await;
            assert_eq!(ledger_count(&scanner).await, 0);
        }
    }

    mod scan {
        use super::*;

        #[tokio::test]
        async fn empty_set_scan_is_noop() {
            let clock = Clock::new(1000);
            let scanner = spawn_scanner(&clock);
            for _ in 0..5 {
                scan_once(&scanner).await;
            }
            assert_eq!(ledger_count(&scanner).await, 0);
        }

        #[tokio::test]
        async fn deployed_collection_discovers_and_evaluates() {
            let (dir, clock, scanner) = single_coll(0).await;

            // file created after the collection => not preexisting => a candidate.
            write(&dir, "new.mcap", b"aaa").await;

            scan_once(&scanner).await;
            assert_eq!(ledger_count(&scanner).await, 0);
            tick(&scanner, &clock, 1).await;
            assert_eq!(ledger_count(&scanner).await, 1);
        }

        // A non-deployed scanner keeps evaluating its existing candidate pool but does
        // NOT discover new files. clear_rules drops the collection from `deployed` but
        // leaves the scanner in place.
        #[tokio::test]
        async fn inactive_scanner_evaluates_but_does_not_discover() {
            let (dir, clock, scanner) = single_coll(10).await;

            // discover one candidate while deployed
            write(&dir, "first.mcap", b"aaa").await;
            scan_once(&scanner).await;

            // inactive scanner should evaluate existing but not discover new files
            scanner.clear_rules().await.unwrap();
            write(&dir, "second.mcap", b"bbb").await;

            let mut rx = subscribe(&scanner).await;
            clock.advance(10);
            scan_once(&scanner).await;
            assert_one_stable(&mut rx, "first.mcap").await;
            // The now-empty inactive collection is no longer active after this tick.
            assert!(!active_collections(&scanner).await.contains(DEFAULT_COLL_ID));
        }

        // An inactive scanner with no remaining candidates is removed from the active
        // collection set on the next scan (get_rules no longer reflects it).
        #[tokio::test]
        async fn inactive_empty_scanner_is_pruned() {
            let (_dir, _clock, scanner) = single_coll(0).await;
            assert_eq!(active_collections(&scanner).await.len(), 1);

            scanner.clear_rules().await.unwrap();
            scan_once(&scanner).await;
            assert!(scanner.get_rules().await.unwrap().is_empty());
        }

        // Two distinct collections matching the same file do NOT share dedup state, so
        // the summed ledger count is 2 (per-collection isolation).
        #[tokio::test]
        async fn distinct_collections_do_not_share_dedup() {
            let dir = dirs::temp("testing").unwrap();
            let glob = format!("{}/*.mcap", dir.path().display());

            let clock = Clock::new(1000);
            let scanner = spawn_scanner(&clock);
            let rulec1 = rule_in_collection("c1", "coll-1", &glob, 0);
            let rulec2 = rule_in_collection("c2", "coll-2", &glob, 0);
            deploy(&scanner, vec![rulec1, rulec2]).await;
            write(&dir, "shared.mcap", b"sss").await;

            let mut rx = subscribe(&scanner).await;

            scan_once(&scanner).await;
            tick(&scanner, &clock, 1).await;
            assert_eq!(ledger_count(&scanner).await, 2);

            // exactly two StableFiles, one per distinct collection (upload_rule_id is
            // stamped from the collection id in observe_file).
            let mut emitted = Vec::new();
            while let Ok(ScanEvent::StableFile(sf)) = rx.try_recv() {
                emitted.push(sf);
            }
            assert_eq!(emitted.len(), 2, "expected exactly two StableFiles");
            let colls: BTreeSet<String> =
                emitted.iter().map(|sf| sf.upload_rule_id.clone()).collect();
            assert_eq!(
                colls,
                BTreeSet::from(["coll-1".to_string(), "coll-2".to_string()])
            );
        }

        // A discovery error in one collection does not prevent a sibling collection
        // from emitting its stable file.
        #[tokio::test]
        async fn scan_isolates_bad_glob_collection_from_emitting_sibling() {
            use crate::scan::collection::CollectionScanner;
            use crate::scan::state::{Config, State};

            let clock = Clock::new(1000);
            let mut single = SingleThreadScanner::new(ScannerArgs {
                now_fn: Arc::new(clock.now_fn()),
                broadcast_capacity: DEFAULT_BROADCAST_CAPACITY,
                state_file: None,
            });

            // --- good collection: a real file, discovered as a candidate at t=1000. ---
            let good_dir = dirs::temp("testing").unwrap();
            let good_glob = format!("{}/*.mcap", good_dir.path().display());
            let good_cfg = Config {
                deployment: deployment("d"),
                rule: rule_in_collection("r-good", "good", &good_glob, 0),
            };
            // build empty (no preexisting), then create the file and discover it so it
            // is a tracked candidate BEFORE the scan under test.
            let mut good = CollectionScanner::from_state(State::new(good_cfg));
            write(&good_dir, "good.mcap", b"aaaa").await;
            good.discover_candidates(clock.now_fn()()).await.unwrap();

            // --- bad collection: a MALFORMED glob that errors at discover time. ---
            let bad_cfg = Config {
                deployment: deployment("d"),
                rule: rule_in_collection("r-bad", "bad", "[", 0),
            };
            // from_state skips the constructor glob, so the bad pattern only bites at
            // scan() time (discover_candidates -> files::glob("[") -> InvalidGlobErr).
            let bad = CollectionScanner::from_state(State::new(bad_cfg));

            single.scanners.insert("good".to_string(), good);
            single.scanners.insert("bad".to_string(), bad);
            single.deployed.insert("good".to_string());
            single.deployed.insert("bad".to_string());

            let mut rx = single.subscribe();

            // advance past the window and run one tick. The bad collection's discover
            // errors, so the good collection still emits.
            clock.advance(1);
            single.scan().await.unwrap();

            // the good collection's StableFile was emitted despite the sibling error.
            let ScanEvent::StableFile(sf) = rx.recv().await.unwrap();
            assert_eq!(stable_name(&sf), "good.mcap".to_string());
            assert_eq!(sf.upload_rule_id, "good".to_string());
            assert!(
                rx.try_recv().is_err(),
                "only the good collection should emit"
            );
        }
    }

    mod prune {
        use super::*;

        // prune(before) with `before` AFTER the recorded first_observed_at drops the
        // ledger entry; with `before` earlier retains it.
        #[tokio::test]
        async fn prune_drops_and_retains() {
            let (dir, clock, scanner) = single_coll(0).await;
            write(&dir, "p.mcap", b"aaa").await;

            // discover at t=1000, evaluate stable at t=1001. first_observed_at == 1000.
            scan_once(&scanner).await;
            tick(&scanner, &clock, 1).await;
            assert_eq!(ledger_count(&scanner).await, 1);

            // retain: before (t=999) <= first_observed_at (1000) => keep.
            scanner
                .prune(chrono::DateTime::from_timestamp(999, 0).unwrap())
                .await
                .unwrap();
            assert_eq!(ledger_count(&scanner).await, 1);

            // drop: before (t=1001) > first_observed_at (1000) => remove.
            scanner
                .prune(chrono::DateTime::from_timestamp(1001, 0).unwrap())
                .await
                .unwrap();
            assert_eq!(ledger_count(&scanner).await, 0);
        }
    }

    mod shutdown {
        use super::*;

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
    }
}
