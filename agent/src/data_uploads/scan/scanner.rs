// standard crates
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

// internal crates
pub use crate::data_uploads::scan::state::{Config, StableFile};
use crate::data_uploads::scan::{
    errors::*,
    rule::{Options, RuleScanner},
    sink::StableFileSink,
    state::{RuleState, ScanSnapshotFile, ScannerSnapshot},
};
use crate::models::{Deployment, FileRule, FileRuleID};
use crate::trace;

// external crates
use chrono::{DateTime, Utc};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

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
    pub sinks: Vec<Arc<dyn StableFileSink>>,
    pub(crate) snapshot_file: Option<ScanSnapshotFile>,
}

impl Default for ScannerArgs {
    fn default() -> Self {
        Self {
            now_fn: Arc::new(Utc::now),
            sinks: Vec::new(),
            snapshot_file: None,
        }
    }
}

pub struct SingleThreadScanner {
    scanners: HashMap<FileRuleID, RuleScanner>,
    deployed: HashSet<FileRuleID>,
    now_fn: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
    sinks: Vec<Arc<dyn StableFileSink>>,
    snapshot_file: Option<ScanSnapshotFile>,
}

impl SingleThreadScanner {
    pub fn new(args: ScannerArgs) -> Result<Self, ScanErr> {
        let persisted = Self::load_snapshot(&args.snapshot_file);
        let scanners = persisted
            .rules
            .iter()
            .map(|(rule_id, state)| {
                (
                    rule_id.clone(),
                    RuleScanner::from_state(state.clone(), Options::default()),
                )
            })
            .collect();
        Ok(Self {
            scanners,
            deployed: persisted.deployed,
            now_fn: args.now_fn,
            sinks: args.sinks,
            snapshot_file: args.snapshot_file,
        })
    }

    fn load_snapshot(state_file: &Option<ScanSnapshotFile>) -> ScannerSnapshot {
        let Some(state_file) = state_file.as_ref() else {
            return ScannerSnapshot {
                rules: HashMap::new(),
                deployed: HashSet::new(),
            };
        };
        state_file.read().as_ref().clone()
    }

    async fn persist_snapshot(&mut self) {
        let Some(state_file) = self.snapshot_file.as_mut() else {
            return;
        };
        let rules: HashMap<FileRuleID, RuleState> = self
            .scanners
            .iter()
            .map(|(rule_id, scanner)| (rule_id.clone(), scanner.state().clone()))
            .collect();
        let snapshot = ScannerSnapshot {
            rules,
            deployed: self.deployed.clone(),
        };
        if let Err(err) = state_file.patch(snapshot).await {
            warn!("scan: failed to persist scanner state: {err}");
        }
    }

    async fn dispatch_stable_files(&self, stable_files: Vec<(StableFile, FileRule)>) {
        let Some((last, rest)) = self.sinks.split_last() else {
            if !stable_files.is_empty() {
                let count = stable_files.len();
                debug!("scan: no sinks attached; {count} stable file(s) not delivered");
            }
            return;
        };
        for (file, rule) in stable_files {
            for sink in rest {
                sink.on_stable_file(file.clone(), &rule).await;
            }
            last.on_stable_file(file, &rule).await;
        }
    }

    #[cfg(feature = "test")]
    async fn get_rules(&self) -> Result<Vec<FileRule>, ScanErr> {
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
            .map(RuleScanner::ledger_count)
            .sum::<usize>();
        Ok(count)
    }

    async fn clear_rules(&mut self) -> Result<(), ScanErr> {
        self.deployed.clear();
        self.persist_snapshot().await;
        info!("scan: cleared all deployed file rules");
        Ok(())
    }

    async fn update_rules(
        &mut self,
        deployment: Deployment,
        rules: Vec<FileRule>,
    ) -> Result<(), ScanErr> {
        let mut deployed: HashSet<FileRuleID> = HashSet::new();
        for rule in rules.iter() {
            if deployed.contains(&rule.id) {
                return Err(ScanErr::DuplicateFileRuleID(DuplicateFileRuleID {
                    file_rule_id: rule.id.clone(),
                    trace: trace!(),
                }));
            }
            deployed.insert(rule.id.clone());
        }

        // Every rule gets a scanner, whether or not it uploads: a retention-only
        // rule still needs its glob walked and its files ledgered so the
        // retention engine has eligibility to act on.
        let now = (self.now_fn)();
        for rule in rules.iter() {
            match self.scanners.get_mut(&rule.id) {
                Some(scanner) => scanner.set_deployment(deployment.clone()),
                None => {
                    let config = Config {
                        deployment: deployment.clone(),
                        rule: rule.clone(),
                    };
                    self.scanners.insert(
                        rule.id.clone(),
                        RuleScanner::new(config, now, Options::default()).await?,
                    );
                }
            }
        }

        self.deployed = deployed;
        self.persist_snapshot().await;

        let count = rules.len();
        let deployment_id = &deployment.id;
        info!("scan: applied {count} rule(s) for deployment {deployment_id}");

        Ok(())
    }

    async fn scan(&mut self) -> Result<(), ScanErr> {
        let mut stable_files = Vec::new();
        let mut inactive_rules = Vec::new();

        let now = (self.now_fn)();

        let active = self.scanners.len();
        let deployed = self.deployed.len();
        debug!("scan: tick over {active} rule(s), {deployed} deployed");

        // evaludate candidates for all scanners
        for (rule_id, scanner) in self.scanners.iter_mut() {
            match scanner.evaluate_candidates(now).await {
                Ok(stable) => {
                    if !stable.is_empty() {
                        let count = stable.len();
                        debug!("scan: rule {rule_id} produced {count} stable file(s)");
                    }
                    let rule = scanner.rule().clone();
                    stable_files.extend(stable.into_iter().map(|file| (file, rule.clone())));
                }
                Err(err) => warn!("scan: evaluate failed for rule {rule_id}: {err}"),
            }

            // discover candidates for deployed scanners
            if self.deployed.contains(rule_id) {
                if let Err(err) = scanner.discover_candidates(now).await {
                    warn!("scan: discover failed for rule {rule_id}: {err}");
                }
            // if the scanner has no candidates, it is inactive
            } else if !scanner.has_candidates() {
                inactive_rules.push(rule_id.clone());
            }
        }

        // prune inactive rule scanners
        let pruned = inactive_rules.len();
        for rule_id in inactive_rules {
            info!("scan: pruned inactive rule {rule_id}");
            self.scanners.remove(&rule_id);
        }

        self.persist_snapshot().await;
        let delivered = stable_files.len();
        self.dispatch_stable_files(stable_files).await;

        debug!(
            "scan: tick complete; {delivered} stable file(s) delivered to sinks, \
             {pruned} inactive rule(s) pruned"
        );

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
        rules: Vec<FileRule>,
    ) -> Result<(), ScanErr>;
    async fn scan(&self) -> Result<(), ScanErr>;
    async fn shutdown(&self) -> Result<(), ScanErr>;
}

pub enum Command {
    ClearRules {
        respond_to: oneshot::Sender<Result<(), ScanErr>>,
    },
    UpdateRules {
        deployment: Box<Deployment>,
        rules: Vec<FileRule>,
        respond_to: oneshot::Sender<Result<(), ScanErr>>,
    },
    Scan {
        respond_to: oneshot::Sender<Result<(), ScanErr>>,
    },
    Shutdown {
        respond_to: oneshot::Sender<Result<(), ScanErr>>,
    },
    #[cfg(feature = "test")]
    GetRules {
        respond_to: oneshot::Sender<Result<Vec<FileRule>, ScanErr>>,
    },
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
            scanner: SingleThreadScanner::new(args)?,
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
    pub async fn get_rules(&self) -> Result<Vec<FileRule>, ScanErr> {
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
        rules: Vec<FileRule>,
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

    async fn shutdown(&self) -> Result<(), ScanErr> {
        self.send_command(|tx| Command::Shutdown { respond_to: tx })
            .await??;
        info!("Scanner shutdown complete");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    // standard crates
    use std::collections::{BTreeSet, HashMap, HashSet};
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::sync::{Arc, Mutex};

    // internal crates
    use super::{Scanner, ScannerArgs, SingleThreadScanner, StableFile, Worker};
    use super::{ScannerExt, StableFileSink};
    use crate::data_uploads::scan::rule::RuleScanner;
    use crate::data_uploads::scan::state::{Config, RuleState, ScanSnapshotFile, ScannerSnapshot};
    use crate::filesys::{dirs, files, Dir, File, PathExt, WriteOptions};
    use crate::models::{Deployment, DplActivity, FileRule, FileRuleSource, FileRuleUpload};

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

    /// Default rule id — the key scanners are held by — for `single_rule` and
    /// tests that redeploy the same scanner map entry.
    const DEFAULT_RULE_ID: &str = "r";

    /// A Deployed deployment with the given id (release_id mirrors the id).
    fn deployment(id: &str) -> Deployment {
        Deployment {
            id: id.to_string(),
            activity_status: DplActivity::Deployed,
            release_id: id.to_string(),
            ..Default::default()
        }
    }

    /// An upload-bearing FileRule pinned to a rule id, glob, and stability
    /// window. Its collection id is derived from the rule id; use
    /// [`rule_in_collection`] when the collection id itself is under test.
    fn upload_rule(rule_id: &str, glob: &str, stability_window_secs: i64) -> FileRule {
        rule_in_collection(
            rule_id,
            &format!("{rule_id}-coll"),
            glob,
            stability_window_secs,
        )
    }

    /// A retention-only FileRule: scanned and ledgered, but never uploaded.
    fn retention_only_rule(rule_id: &str, glob: &str, stability_window_secs: i64) -> FileRule {
        FileRule {
            upload: None,
            ..upload_rule(rule_id, glob, stability_window_secs)
        }
    }

    /// Build a FileRule with a pinned `upload_collection_id` (plus rule id/glob/window).
    fn rule_in_collection(
        rule_id: &str,
        upload_collection_id: &str,
        glob: &str,
        stability_window_secs: i64,
    ) -> FileRule {
        FileRule {
            id: rule_id.to_string(),
            upload: Some(FileRuleUpload {
                upload_collection_id: upload_collection_id.to_string(),
                ..Default::default()
            }),
            source: FileRuleSource {
                glob: glob.to_string(),
                stability_window_secs,
            },
            ..Default::default()
        }
    }

    /// A recording [`StableFileSink`]: captures every delivered
    /// `(StableFile, FileRule)` pair for assertions after a tick. Clone-shared —
    /// the scanner holds one handle, the test another.
    #[derive(Clone, Default)]
    pub struct RecordingSink {
        events: Arc<Mutex<Vec<(StableFile, FileRule)>>>,
    }

    impl RecordingSink {
        fn new() -> Self {
            Self::default()
        }

        /// The recorded `(StableFile, FileRule)` pairs, in delivery order.
        fn events(&self) -> Vec<(StableFile, FileRule)> {
            self.events.lock().unwrap().clone()
        }

        /// The recorded stable files' names, in delivery order.
        fn names(&self) -> Vec<String> {
            self.events().iter().map(|(f, _)| stable_name(f)).collect()
        }

        /// The number of recorded deliveries.
        fn count(&self) -> usize {
            self.events.lock().unwrap().len()
        }

        /// Assert exactly one delivery so far, named `name`. The caller supplies
        /// the WHY exactly-one holds at its own site.
        fn assert_one_stable(&self, name: &str) {
            assert_eq!(self.names(), vec![name.to_string()]);
        }
    }

    impl StableFileSink for RecordingSink {
        fn on_stable_file<'a>(
            &'a self,
            file: StableFile,
            rule: &'a FileRule,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
            Box::pin(async move {
                // yield before recording: proves the scan tick awaits the sink
                // future to completion rather than fire-and-forgetting it.
                tokio::task::yield_now().await;
                self.events.lock().unwrap().push((file, rule.clone()));
            })
        }
    }

    /// Spawn a scanner actor with a deterministic injected clock and a discarded
    /// recording sink, for tests that only assert ledger/snapshot state. The
    /// sink is attached anyway so every test runs the production shape — the
    /// app always wires at least one sink — and the dispatch path executes even
    /// where its output goes unasserted. Zero-sink dispatch is pinned by the
    /// dedicated `scan_with_no_sinks_does_not_error` test.
    fn spawn_scanner(clock: &Clock) -> Scanner {
        let (scanner, _sink) = spawn_scanner_with_sink(clock);
        scanner
    }

    /// Spawn a scanner actor with a deterministic injected clock and one
    /// recording sink; returns the test's sink handle alongside the scanner.
    fn spawn_scanner_with_sink(clock: &Clock) -> (Scanner, RecordingSink) {
        let sink = RecordingSink::new();
        let scanner = spawn_scanner_with_sinks(clock, vec![Arc::new(sink.clone())]);
        (scanner, sink)
    }

    /// Spawn a scanner actor with a deterministic injected clock and the given
    /// sinks.
    fn spawn_scanner_with_sinks(clock: &Clock, sinks: Vec<Arc<dyn StableFileSink>>) -> Scanner {
        let (scanner, _h) = Scanner::spawn(
            64,
            ScannerArgs {
                now_fn: Arc::new(clock.now_fn()),
                sinks,
                snapshot_file: None,
            },
        )
        .unwrap();
        scanner
    }

    /// temp dir + `*.mcap` glob + spawned scanner + one deployed upload-bearing rule
    /// keyed [`DEFAULT_RULE_ID`] (deployment "d", window `window`). Returns (dir,
    /// clock, scanner). Hold `dir` to keep the temp tree alive.
    async fn single_rule(window: i64) -> (dirs::TempDir, Clock, Scanner) {
        let dir = dirs::temp("testing").unwrap();
        let glob = format!("{}/*.mcap", dir.path().display());
        let clock = Clock::new(1000);
        let scanner = spawn_scanner(&clock);
        deploy_single_rule(&scanner, &glob, window).await;
        (dir, clock, scanner)
    }

    /// [`single_rule`], but with a recording sink attached at spawn.
    async fn single_rule_with_sink(window: i64) -> (dirs::TempDir, Clock, Scanner, RecordingSink) {
        let dir = dirs::temp("testing").unwrap();
        let glob = format!("{}/*.mcap", dir.path().display());
        let clock = Clock::new(1000);
        let (scanner, sink) = spawn_scanner_with_sink(&clock);
        deploy_single_rule(&scanner, &glob, window).await;
        (dir, clock, scanner, sink)
    }

    /// Deploy one upload-bearing rule keyed [`DEFAULT_RULE_ID`] under
    /// deployment "d".
    async fn deploy_single_rule(scanner: &Scanner, glob: &str, window: i64) {
        scanner
            .update_rules(
                deployment("d"),
                vec![upload_rule(DEFAULT_RULE_ID, glob, window)],
            )
            .await
            .unwrap();
    }

    /// The set of rule ids currently held by the scanner.
    fn rule_ids(rules: &[FileRule]) -> BTreeSet<String> {
        rules.iter().map(|r| r.id.clone()).collect()
    }

    /// The set of `upload_collection_id` values currently held by the scanner.
    fn collection_ids(rules: &[FileRule]) -> BTreeSet<String> {
        rules
            .iter()
            .filter_map(|r| r.upload.as_ref())
            .map(|u| u.upload_collection_id.clone())
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
    async fn deploy(scanner: &Scanner, rules: Vec<FileRule>) {
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

    /// The number of entries in the scanner's ledger.
    async fn ledger_count(scanner: &Scanner) -> usize {
        scanner.get_ledger_count().await.unwrap()
    }

    /// Rule ids with live scanner state — the scanner map's keys — including
    /// inactive legacy scanners that are still draining candidates.
    async fn active_rule_ids(scanner: &Scanner) -> BTreeSet<String> {
        rule_ids(&scanner.get_rules().await.unwrap())
    }

    async fn state_file(file: &File) -> ScanSnapshotFile {
        ScanSnapshotFile::new_with_default(file.clone(), ScannerSnapshot::default())
            .await
            .unwrap()
    }

    async fn spawn_persisted(clock: &Clock, file: &File) -> Scanner {
        let (scanner, _h) = Scanner::spawn(
            64,
            ScannerArgs {
                now_fn: Arc::new(clock.now_fn()),
                sinks: vec![Arc::new(RecordingSink::new())],
                snapshot_file: Some(state_file(file).await),
            },
        )
        .unwrap();
        scanner
    }

    async fn read_snapshot(file: &File) -> ScannerSnapshot {
        files::read_json(file).await.unwrap()
    }

    fn mcap_glob(dir: &Dir) -> String {
        format!("{}/*.mcap", dir.path().display())
    }

    struct PersistedScannerFixture {
        dir: dirs::TempDir,
        clock: Clock,
        scanner: Scanner,
        state_path: File,
    }

    async fn persisted_rule(window: i64) -> PersistedScannerFixture {
        let dir = dirs::temp("testing").unwrap();
        let state_path = dir.file("scanner.json");
        let clock = Clock::new(1000);
        let scanner = spawn_persisted(&clock, &state_path).await;
        let glob = mcap_glob(&dir);
        let rule = upload_rule(DEFAULT_RULE_ID, &glob, window);
        deploy(&scanner, vec![rule]).await;
        PersistedScannerFixture {
            dir,
            clock,
            scanner,
            state_path,
        }
    }

    mod construction {
        use super::*;

        #[tokio::test]
        async fn worker_new_and_scanner_new_round_trip() {
            let (tx, rx) = mpsc::channel(64);
            let single = SingleThreadScanner::new(ScannerArgs::default()).unwrap();
            let worker = Worker::new(single, rx);
            let handle = tokio::spawn(worker.run());

            let scanner = Scanner::new(tx);

            // the hand-built actor answers commands: empty ledger.
            assert_eq!(scanner.get_ledger_count().await.unwrap(), 0);

            // and shuts down cleanly; later commands then error on a closed channel.
            scanner.shutdown().await.unwrap();
            handle.await.unwrap();
            let err = scanner.get_ledger_count().await.unwrap_err();
            assert!(matches!(
                err,
                crate::data_uploads::scan::ScanErr::SendActorMessageErr(_)
            ));
        }

        #[tokio::test]
        async fn missing_state_file_starts_fresh() {
            let fxtr = persisted_rule(0).await;
            write(&fxtr.dir, "a.mcap", b"aaaa").await;
            scan_once(&fxtr.scanner).await;
            tick(&fxtr.scanner, &fxtr.clock, 1).await;
            assert_eq!(ledger_count(&fxtr.scanner).await, 1);

            assert!(fxtr.state_path.exists());
            let snapshot = read_snapshot(&fxtr.state_path).await;
            assert!(snapshot.rules.contains_key(DEFAULT_RULE_ID));
        }

        #[tokio::test]
        async fn corrupt_state_file_starts_fresh() {
            let dir = dirs::temp("testing").unwrap();
            let state_path = dir.file("scanner.json");
            files::seed(&state_path, "not json").await;

            let clock = Clock::new(1000);
            let scanner = spawn_persisted(&clock, &state_path).await;
            let rule = upload_rule(DEFAULT_RULE_ID, &mcap_glob(&dir), 0);
            deploy(&scanner, vec![rule]).await;
            write(&dir, "a.mcap", b"aaaa").await;
            scan_once(&scanner).await;
            tick(&scanner, &clock, 1).await;
            assert_eq!(ledger_count(&scanner).await, 1);

            let snapshot = read_snapshot(&state_path).await;
            let state = snapshot.rules.get(DEFAULT_RULE_ID).unwrap();
            assert_eq!(state.ledger.len(), 1);
        }

        /// A snapshot written by a pre-rule-keying agent has a `collections`
        /// field instead of `rules`. Both were `HashMap<String, _>`, so without
        /// the field rename it would deserialize cleanly and attach one rule's
        /// ledger to a different rule. It must fail to parse and start fresh.
        #[tokio::test]
        async fn stale_collection_keyed_state_file_starts_fresh() {
            let dir = dirs::temp("testing").unwrap();
            let state_path = dir.file("scanner.json");
            files::seed(
                &state_path,
                r#"{"collections":{"coll-1":{"cfg":{"deployment":{},"rule":{}},
                   "preexisting":{},"candidates":{},"ledger":{}}},
                   "deployed":["coll-1"]}"#,
            )
            .await;

            let clock = Clock::new(1000);
            let scanner = spawn_persisted(&clock, &state_path).await;

            // nothing carried over from the stale snapshot.
            assert!(scanner.get_rules().await.unwrap().is_empty());

            // and the scanner is usable: a fresh rule-keyed snapshot is written.
            let rule = upload_rule(DEFAULT_RULE_ID, &mcap_glob(&dir), 0);
            deploy(&scanner, vec![rule]).await;
            write(&dir, "a.mcap", b"aaaa").await;
            scan_once(&scanner).await;
            tick(&scanner, &clock, 1).await;

            let snapshot = read_snapshot(&state_path).await;
            assert!(snapshot.rules.contains_key(DEFAULT_RULE_ID));
        }

        #[tokio::test]
        async fn existing_state_file_restores_scanner() {
            let dir = dirs::temp("testing").unwrap();
            let state_path = dir.file("scanner.json");
            let rule_state = RuleState::new(Config {
                deployment: deployment("d"),
                rule: upload_rule(DEFAULT_RULE_ID, "/none/*.mcap", 0),
            });
            let expected = ScannerSnapshot {
                rules: HashMap::from([(DEFAULT_RULE_ID.to_string(), rule_state)]),
                deployed: HashSet::from([DEFAULT_RULE_ID.to_string()]),
            };
            let mut snapshot_file = state_file(&state_path).await;
            snapshot_file.patch(expected.clone()).await.unwrap();

            let scanner = SingleThreadScanner::new(ScannerArgs {
                snapshot_file: Some(snapshot_file),
                sinks: vec![Arc::new(RecordingSink::new())],
                ..ScannerArgs::default()
            })
            .unwrap();

            assert_eq!(scanner.deployed, expected.deployed);
            assert_eq!(
                scanner.scanners.get(DEFAULT_RULE_ID).unwrap().state(),
                expected.rules.get(DEFAULT_RULE_ID).unwrap()
            );
        }
    }

    mod load_snapshot {
        use super::*;

        #[tokio::test]
        async fn loads_existing_snapshot() {
            let dir = dirs::temp("testing").unwrap();
            let state_path = dir.file("scanner.json");
            let config = Config {
                deployment: deployment("d"),
                rule: upload_rule(DEFAULT_RULE_ID, "/none/*.mcap", 0),
            };
            let expected = ScannerSnapshot {
                rules: HashMap::from([(DEFAULT_RULE_ID.to_string(), RuleState::new(config))]),
                deployed: HashSet::from([DEFAULT_RULE_ID.to_string()]),
            };
            let mut snapshot_file = state_file(&state_path).await;
            snapshot_file.patch(expected.clone()).await.unwrap();

            let loaded = SingleThreadScanner::load_snapshot(&Some(snapshot_file));

            assert_eq!(loaded, expected);
        }
    }

    mod persist_snapshot {
        use super::*;

        // internal crates
        use crate::data_uploads::scan::rule::Options;

        #[tokio::test]
        async fn writes_current_snapshot() {
            let dir = dirs::temp("testing").unwrap();
            let state_path = dir.file("scanner.json");
            let clock = Clock::new(1000);
            let config = Config {
                deployment: deployment("d"),
                rule: upload_rule(DEFAULT_RULE_ID, "/none/*.mcap", 0),
            };
            let rule_state = RuleState::new(config);
            let expected = ScannerSnapshot {
                rules: HashMap::from([(DEFAULT_RULE_ID.to_string(), rule_state.clone())]),
                deployed: HashSet::from([DEFAULT_RULE_ID.to_string()]),
            };
            let mut scanner = SingleThreadScanner::new(ScannerArgs {
                now_fn: Arc::new(clock.now_fn()),
                sinks: vec![Arc::new(RecordingSink::new())],
                snapshot_file: Some(state_file(&state_path).await),
            })
            .unwrap();
            scanner.scanners.insert(
                DEFAULT_RULE_ID.to_string(),
                RuleScanner::from_state(rule_state, Options::default()),
            );
            scanner.deployed.insert(DEFAULT_RULE_ID.to_string());

            scanner.persist_snapshot().await;

            assert_eq!(read_snapshot(&state_path).await, expected);
        }

        #[tokio::test]
        async fn write_failure_is_swallowed() {
            let dir = dirs::temp("testing").unwrap();
            let state_path = dir.file("scanner.json");
            let rule_state = RuleState::new(Config {
                deployment: deployment("d"),
                rule: upload_rule(DEFAULT_RULE_ID, "/none/*.mcap", 0),
            });
            let mut scanner = SingleThreadScanner::new(ScannerArgs {
                snapshot_file: Some(state_file(&state_path).await),
                sinks: vec![Arc::new(RecordingSink::new())],
                ..ScannerArgs::default()
            })
            .unwrap();
            scanner.scanners.insert(
                DEFAULT_RULE_ID.to_string(),
                RuleScanner::from_state(rule_state, Options::default()),
            );
            scanner.deployed.insert(DEFAULT_RULE_ID.to_string());

            files::delete(&state_path).await.unwrap();
            dirs::create(&Dir::new(state_path.path().clone()))
                .await
                .unwrap();

            scanner.persist_snapshot().await;

            let cached = scanner.snapshot_file.as_ref().unwrap().read();
            assert_eq!(cached.as_ref(), &ScannerSnapshot::default());
            assert!(scanner.scanners.contains_key(DEFAULT_RULE_ID));
            assert!(scanner.deployed.contains(DEFAULT_RULE_ID));
        }
    }

    mod sinks {
        use super::*;

        // a delivered StableFile carries the expected payload; dispatch is
        // awaited inside scan(), so the sink is populated when the tick returns.
        #[tokio::test]
        async fn sink_receives_stable_file_payload() {
            let dir = dirs::temp("testing").unwrap();
            let glob = format!("{}/*.mcap", dir.path().display());

            let clock = Clock::new(1000);
            let (scanner, sink) = spawn_scanner_with_sink(&clock);
            let rule = upload_rule("rule-1", &glob, 0);
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
                file_rule_id: "rule-1".to_string(),
            };

            scan_once(&scanner).await; // discover
            tick(&scanner, &clock, 1).await; // evaluate => deliver

            let events = sink.events();
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].0, expected);
        }

        /// A retention-only rule runs the full scan pipeline — glob, stability
        /// window, ledger, deliver — and its delivery carries `upload: None` so
        /// sinks know not to mint an upload job.
        #[tokio::test]
        async fn retention_only_rule_scans_and_delivers_without_upload() {
            let dir = dirs::temp("testing").unwrap();
            let clock = Clock::new(1000);
            let (scanner, sink) = spawn_scanner_with_sink(&clock);
            deploy(
                &scanner,
                vec![retention_only_rule(DEFAULT_RULE_ID, &mcap_glob(&dir), 0)],
            )
            .await;

            write(&dir, "keep.mcap", b"aaa").await;
            scan_once(&scanner).await; // discover
            tick(&scanner, &clock, 1).await; // evaluate => deliver

            let events = sink.events();
            assert_eq!(events.len(), 1);
            let (file, rule) = &events[0];
            assert_eq!(stable_name(file), "keep.mcap");
            assert_eq!(file.file_rule_id, DEFAULT_RULE_ID);
            assert_eq!(rule.upload, None);

            // it reached the ledger too — the retention engine reads from there.
            assert_eq!(ledger_count(&scanner).await, 1);
        }

        /// An upload-bearing rule's delivery carries its upload block, so the
        /// two arms of the upload sink's gate are distinguishable at the source.
        #[tokio::test]
        async fn upload_rule_delivers_its_upload_block() {
            let (dir, clock, scanner, sink) = single_rule_with_sink(0).await;
            write(&dir, "up.mcap", b"aaa").await;
            scan_once(&scanner).await;
            tick(&scanner, &clock, 1).await;

            let events = sink.events();
            assert_eq!(events.len(), 1);
            let rule = events[0].1.clone();
            assert_eq!(
                rule.upload.map(|u| u.upload_collection_id),
                Some(format!("{DEFAULT_RULE_ID}-coll"))
            );
        }

        // scan() producing stable files with NO sinks attached does not error.
        // Zero sinks cannot occur in the app (init always wires the upload
        // sink) but is ScannerArgs::default(); this is the one test that
        // exercises the empty-dispatch branch.
        #[tokio::test]
        async fn scan_with_no_sinks_does_not_error() {
            let dir = dirs::temp("testing").unwrap();
            let clock = Clock::new(1000);
            let scanner = spawn_scanner_with_sinks(&clock, Vec::new());
            deploy(
                &scanner,
                vec![upload_rule(DEFAULT_RULE_ID, &mcap_glob(&dir), 0)],
            )
            .await;
            write(&dir, "nosink.mcap", b"aaa").await;

            scan_once(&scanner).await;
            tick(&scanner, &clock, 1).await; // delivers to zero sinks, must not error
            assert_eq!(ledger_count(&scanner).await, 1);
        }

        /// Every sink in the vec receives every stable file (PR 3b adds a
        /// second production sink; the fan-out must already hold).
        #[tokio::test]
        async fn every_sink_receives_every_stable_file() {
            let dir = dirs::temp("testing").unwrap();
            let glob = format!("{}/*.mcap", dir.path().display());
            let clock = Clock::new(1000);
            let first = RecordingSink::new();
            let second = RecordingSink::new();
            let scanner = spawn_scanner_with_sinks(
                &clock,
                vec![Arc::new(first.clone()), Arc::new(second.clone())],
            );
            deploy_single_rule(&scanner, &glob, 0).await;

            write(&dir, "fanout.mcap", b"aaa").await;
            scan_once(&scanner).await;
            tick(&scanner, &clock, 1).await;

            first.assert_one_stable("fanout.mcap");
            second.assert_one_stable("fanout.mcap");
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

        #[tokio::test]
        async fn clear_rules_persists_snapshot() {
            let fxtr = persisted_rule(0).await;
            let before = read_snapshot(&fxtr.state_path).await;
            assert!(before.deployed.contains(DEFAULT_RULE_ID));

            fxtr.scanner.clear_rules().await.unwrap();

            let after = read_snapshot(&fxtr.state_path).await;
            assert!(after.deployed.is_empty());
            assert!(after.rules.contains_key(DEFAULT_RULE_ID));
        }

        // clear_rules empties `deployed` but leaves scanners in place; a subsequent
        // scan still evaluates their remaining candidates. Here the candidate goes
        // stable while inactive (clear does not immediately drop the scanner or its
        // pool).
        #[tokio::test]
        async fn clear_rules_still_evaluates_remaining_candidates() {
            let (dir, clock, scanner, sink) = single_rule_with_sink(0).await;
            write(&dir, "drain.mcap", b"aaa").await;

            // discover a candidate while deployed.
            scan_once(&scanner).await;
            scanner.clear_rules().await.unwrap();

            // The rule's scanner remains active while its existing candidate drains.
            assert_eq!(active_rule_ids(&scanner).await.len(), 1);

            // the evaluating scan delivers the drained StableFile — exactly one
            // event total, since the discover tick emitted nothing.
            tick(&scanner, &clock, 1).await;
            sink.assert_one_stable("drain.mcap");

            // the now-empty inactive scanner was pruned this tick.
            assert!(scanner.get_rules().await.unwrap().is_empty());
        }

        // An inactive scanner whose candidate becomes Unstable (deleted) has no remaining
        // candidates and is pruned on the next scan (drain-then-prune).
        #[tokio::test]
        async fn clear_rules_drains_unstable_candidate_then_prunes() {
            let (dir, clock, scanner) = single_rule(5).await;
            let file = write(&dir, "drain.mcap", b"aaa").await;

            // discover a candidate while deployed.
            scan_once(&scanner).await;
            scanner.clear_rules().await.unwrap();
            assert_eq!(active_rule_ids(&scanner).await.len(), 1);

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

        // A rule set with two rules sharing one rule id is rejected BEFORE any
        // state mutation.
        #[tokio::test]
        async fn update_rules_duplicate_rule_id_errors() {
            let clock = Clock::new(1000);
            let scanner = spawn_scanner(&clock);

            // seed a known-good single rule first.
            deploy(&scanner, vec![upload_rule("r0", "/none/*.mcap", 0)]).await;
            let before = active_rule_ids(&scanner).await;

            // push a set with a duplicate rule id => error.
            let rules = vec![
                upload_rule("dup", "/a/*.mcap", 0),
                upload_rule("dup", "/b/*.mcap", 0),
            ];
            let err = scanner
                .update_rules(deployment("d"), rules)
                .await
                .unwrap_err();
            assert!(matches!(
                err,
                crate::data_uploads::scan::ScanErr::DuplicateFileRuleID(_)
            ));

            // No state mutated: the existing active rule is untouched and the
            // duplicate set was not applied.
            assert_eq!(active_rule_ids(&scanner).await, before);
        }

        // Collection ids are no longer keys, so two rules may share one. Under the
        // old collection-keyed scanner this was a hard error; now both rules get
        // their own scanner.
        #[tokio::test]
        async fn update_rules_allows_shared_collection_id() {
            let clock = Clock::new(1000);
            let scanner = spawn_scanner(&clock);

            let rules = vec![
                rule_in_collection("r1", "shared", "/a/*.mcap", 0),
                rule_in_collection("r2", "shared", "/b/*.mcap", 0),
            ];
            deploy(&scanner, rules).await;

            // two scanners, one per rule id...
            assert_eq!(
                active_rule_ids(&scanner).await,
                BTreeSet::from(["r1".to_string(), "r2".to_string()])
            );
            // ...both pointing at the same upload collection.
            assert_eq!(
                collection_ids(&scanner.get_rules().await.unwrap()),
                BTreeSet::from(["shared".to_string()])
            );
        }

        // Pushing a new rule id creates an active scanner reflected in get_rules.
        #[tokio::test]
        async fn update_rules_creates_scanner() {
            let clock = Clock::new(1000);
            let scanner = spawn_scanner(&clock);
            let rule = upload_rule(DEFAULT_RULE_ID, "/none/*.mcap", 0);
            deploy(&scanner, vec![rule]).await;
            assert_eq!(
                active_rule_ids(&scanner).await,
                BTreeSet::from([DEFAULT_RULE_ID.to_string()])
            );
            assert_eq!(
                rule_ids(&scanner.get_rules().await.unwrap()),
                BTreeSet::from(["r".to_string()])
            );
        }

        // A retention-only rule (no upload block) still gets a scanner: its glob
        // must be walked so the retention engine has files to act on. Unreachable
        // with the v0.4 wire adapter (which always produces an upload block), but
        // the pipeline must be capable of it ahead of the v0.5 wire flip.
        #[tokio::test]
        async fn update_rules_scans_rule_without_upload() {
            let clock = Clock::new(1000);
            let scanner = spawn_scanner(&clock);

            deploy(
                &scanner,
                vec![retention_only_rule(DEFAULT_RULE_ID, "/none/*.mcap", 0)],
            )
            .await;

            assert_eq!(
                active_rule_ids(&scanner).await,
                BTreeSet::from([DEFAULT_RULE_ID.to_string()])
            );
            let rules = scanner.get_rules().await.unwrap();
            assert_eq!(rules.len(), 1);
            assert!(rules[0].upload.is_none());
        }

        #[tokio::test]
        async fn update_rules_persists_snapshot() {
            let dir = dirs::temp("testing").unwrap();
            let state_path = dir.file("scanner.json");
            let clock = Clock::new(1000);
            let scanner = spawn_persisted(&clock, &state_path).await;
            assert_eq!(read_snapshot(&state_path).await, ScannerSnapshot::default());

            let rule = upload_rule(DEFAULT_RULE_ID, "/none/*.mcap", 0);
            deploy(&scanner, vec![rule]).await;

            let snapshot = read_snapshot(&state_path).await;
            assert!(snapshot.deployed.contains(DEFAULT_RULE_ID));
            assert!(snapshot.rules.contains_key(DEFAULT_RULE_ID));
        }

        // Re-pushing the SAME rule id keeps the existing scanner and its ledger
        // (no re-report of an already-reported file) while refreshing the
        // deployment stamped onto subsequent stable files. A rule's content is
        // immutable per id, so the deployment is all a re-push can change.
        #[tokio::test]
        async fn update_rules_refreshes_deployment_carrying_state() {
            let dir = dirs::temp("testing").unwrap();
            let glob = format!("{}/*.mcap", dir.path().display());

            let clock = Clock::new(1000);
            let (scanner, sink) = spawn_scanner_with_sink(&clock);
            deploy(&scanner, vec![upload_rule(DEFAULT_RULE_ID, &glob, 0)]).await;

            // file appears after creation and goes stable under deployment "d".
            write(&dir, "carry.mcap", b"ccc").await;
            scan_once(&scanner).await;
            tick(&scanner, &clock, 1).await;
            assert_eq!(ledger_count(&scanner).await, 1);
            let delivered_before_repush = sink.count();

            // the same rule arrives again under a new deployment.
            scanner
                .update_rules(
                    deployment("d2"),
                    vec![upload_rule(DEFAULT_RULE_ID, &glob, 0)],
                )
                .await
                .unwrap();

            // the ledger carried: no re-report of the already-reported file.
            tick(&scanner, &clock, 1).await;
            assert_eq!(ledger_count(&scanner).await, 1);
            assert_eq!(
                sink.count(),
                delivered_before_repush,
                "carried dedup state must not re-deliver the already-reported file"
            );
            assert_eq!(scanner.get_rules().await.unwrap().len(), 1);

            // a file arriving after the re-push is stamped with the new deployment.
            write(&dir, "after.mcap", b"aaa").await;
            scan_once(&scanner).await;
            tick(&scanner, &clock, 1).await;
            let (file, _) = sink.events().last().unwrap().clone();
            assert_eq!(stable_name(&file), "after.mcap");
            assert_eq!(file.deployment_id, "d2");
        }

        // The deployed set is replaced on each update_rules: after [A,B] then [C], only C
        // discovers; A and B drain-and-prune on subsequent scans.
        #[tokio::test]
        async fn update_rules_replaces_deployed_set() {
            let clock = Clock::new(1000);
            let scanner = spawn_scanner(&clock);

            let rules = vec![
                upload_rule("a", "/none/*.mcap", 0),
                upload_rule("b", "/none/*.mcap", 0),
            ];
            deploy(&scanner, rules).await;

            deploy(&scanner, vec![upload_rule("c", "/none/*.mcap", 0)]).await;

            // A and B are no longer deployed and have no candidates, so the next scan
            // removes them from the active scanner set.
            scan_once(&scanner).await;
            assert_eq!(
                active_rule_ids(&scanner).await,
                BTreeSet::from(["c".to_string()])
            );
        }

        // Replacing rule A with rule B keeps A's scanner alive long enough to
        // drain its existing candidates, while only B discovers newly added files.
        #[tokio::test]
        async fn update_rules_keeps_legacy_scanner_until_candidates_drain() {
            let dir = dirs::temp("testing").unwrap();
            let glob = format!("{}/*.mcap", dir.path().display());
            let clock = Clock::new(1000);
            let (scanner, sink) = spawn_scanner_with_sink(&clock);

            deploy(&scanner, vec![upload_rule("legacy-rule", &glob, 10)]).await;
            write(&dir, "legacy.mcap", b"legacy").await;
            scan_once(&scanner).await;

            deploy(&scanner, vec![upload_rule("current-rule", &glob, 10)]).await;
            // The deployed rule and the draining legacy rule are both active.
            assert_eq!(
                active_rule_ids(&scanner).await,
                BTreeSet::from(["current-rule".to_string(), "legacy-rule".to_string()])
            );

            write(&dir, "current.mcap", b"current").await;
            scan_once(&scanner).await;
            tick(&scanner, &clock, 10).await;

            // the earlier ticks only discovered (window 10), so these are the
            // first and only deliveries.
            let delivered: BTreeSet<(String, String)> = sink
                .events()
                .iter()
                .map(|(stable, _)| (stable.file_rule_id.clone(), stable_name(stable)))
                .collect();
            let event1 = ("current-rule".to_string(), "current.mcap".to_string());
            let event2 = ("legacy-rule".to_string(), "legacy.mcap".to_string());
            assert_eq!(delivered, BTreeSet::from([event1, event2]));
            // The legacy rule is no longer active once its candidate pool drains.
            assert_eq!(
                active_rule_ids(&scanner).await,
                BTreeSet::from(["current-rule".to_string()])
            );
        }

        // A re-push of the same rule must NOT swallow files that appeared since
        // the last scan tick. update_rules runs on every sync success; if it
        // re-snapshotted preexisting (as it once did), a file appearing in the
        // window between a scan tick and a sync would be classified preexisting
        // and silently never uploaded.
        #[tokio::test]
        async fn update_rules_repush_does_not_swallow_new_files() {
            let (dir, clock, scanner) = single_rule(0).await;
            let glob = format!("{}/*.mcap", dir.path().display());

            // a file appears after the scanner was created, before any scan tick...
            write(&dir, "late.mcap", b"aaa").await;

            // ...and a sync re-pushes the same rule before the next tick.
            deploy(&scanner, vec![upload_rule(DEFAULT_RULE_ID, &glob, 0)]).await;

            // the file is still discovered and reported.
            scan_once(&scanner).await;
            tick(&scanner, &clock, 100).await;
            assert_eq!(ledger_count(&scanner).await, 1);
        }
    }

    mod scan {
        use super::*;

        // internal crates
        use crate::data_uploads::scan::rule::Options;

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
        async fn scan_persists_snapshot() {
            let fxtr = persisted_rule(10).await;
            let file = write(&fxtr.dir, "candidate.mcap", b"aaaa").await;

            scan_once(&fxtr.scanner).await;

            let snapshot = read_snapshot(&fxtr.state_path).await;
            let state = snapshot.rules.get(DEFAULT_RULE_ID).unwrap();
            assert!(state.candidates.contains_key(&file));
        }

        #[tokio::test]
        async fn deployed_rule_discovers_and_evaluates() {
            let (dir, clock, scanner) = single_rule(0).await;

            // file created after the scanner => not preexisting => a candidate.
            write(&dir, "new.mcap", b"aaa").await;

            scan_once(&scanner).await;
            assert_eq!(ledger_count(&scanner).await, 0);
            tick(&scanner, &clock, 1).await;
            assert_eq!(ledger_count(&scanner).await, 1);
        }

        // A non-deployed scanner keeps evaluating its existing candidate pool but does
        // NOT discover new files. clear_rules drops the rule from `deployed` but
        // leaves the scanner in place.
        #[tokio::test]
        async fn inactive_scanner_evaluates_but_does_not_discover() {
            let (dir, clock, scanner, sink) = single_rule_with_sink(10).await;

            // discover one candidate while deployed
            write(&dir, "first.mcap", b"aaa").await;
            scan_once(&scanner).await;

            // inactive scanner should evaluate existing but not discover new files
            scanner.clear_rules().await.unwrap();
            write(&dir, "second.mcap", b"bbb").await;

            clock.advance(10);
            scan_once(&scanner).await;
            // exactly one delivery across all ticks: second.mcap was never
            // discovered, and first.mcap only went stable this tick.
            sink.assert_one_stable("first.mcap");
            // The now-empty inactive rule is no longer active after this tick.
            assert!(!active_rule_ids(&scanner).await.contains(DEFAULT_RULE_ID));
        }

        // An inactive scanner with no remaining candidates is removed from the active
        // rule set on the next scan (get_rules no longer reflects it).
        #[tokio::test]
        async fn inactive_empty_scanner_is_pruned() {
            let (_dir, _clock, scanner) = single_rule(0).await;
            assert_eq!(active_rule_ids(&scanner).await.len(), 1);

            scanner.clear_rules().await.unwrap();
            scan_once(&scanner).await;
            assert!(scanner.get_rules().await.unwrap().is_empty());
        }

        // Two distinct rules matching the same file do NOT share dedup state, so
        // the summed ledger count is 2 (per-rule isolation).
        #[tokio::test]
        async fn distinct_rules_do_not_share_dedup() {
            let dir = dirs::temp("testing").unwrap();
            let glob = format!("{}/*.mcap", dir.path().display());

            let clock = Clock::new(1000);
            let (scanner, sink) = spawn_scanner_with_sink(&clock);
            let rules = vec![upload_rule("c1", &glob, 0), upload_rule("c2", &glob, 0)];
            deploy(&scanner, rules).await;
            write(&dir, "shared.mcap", b"sss").await;

            scan_once(&scanner).await;
            tick(&scanner, &clock, 1).await;
            assert_eq!(ledger_count(&scanner).await, 2);

            // exactly two StableFiles, one per distinct rule.
            let delivered = sink.events();
            assert_eq!(delivered.len(), 2, "expected exactly two StableFiles");
            let rule_ids: BTreeSet<String> = delivered
                .iter()
                .map(|(sf, _)| sf.file_rule_id.clone())
                .collect();
            assert_eq!(
                rule_ids,
                BTreeSet::from(["c1".to_string(), "c2".to_string()])
            );
        }

        // A discovery error in one rule does not prevent a sibling rule from
        // emitting its stable file.
        #[tokio::test]
        async fn scan_isolates_bad_glob_rule_from_emitting_sibling() {
            let clock = Clock::new(1000);
            let sink = RecordingSink::new();
            let mut single = SingleThreadScanner::new(ScannerArgs {
                now_fn: Arc::new(clock.now_fn()),
                sinks: vec![Arc::new(sink.clone())],
                snapshot_file: None,
            })
            .unwrap();

            // --- good rule: a real file, discovered as a candidate at t=1000. ---
            let good_dir = dirs::temp("testing").unwrap();
            let good_glob = format!("{}/*.mcap", good_dir.path().display());
            let good_cfg = Config {
                deployment: deployment("d"),
                rule: upload_rule("r-good", &good_glob, 0),
            };
            // build empty (no preexisting), then create the file and discover it so it
            // is a tracked candidate BEFORE the scan under test.
            let mut good = RuleScanner::from_state(RuleState::new(good_cfg), Options::default());
            write(&good_dir, "good.mcap", b"aaaa").await;
            good.discover_candidates(clock.now_fn()()).await.unwrap();

            // --- bad rule: a MALFORMED glob that errors at discover time. ---
            let bad_cfg = Config {
                deployment: deployment("d"),
                rule: upload_rule("r-bad", "[", 0),
            };
            // from_state skips the constructor glob, so the bad pattern only bites at
            // scan() time (discover_candidates -> files::glob("[") -> InvalidGlobErr).
            let bad = RuleScanner::from_state(RuleState::new(bad_cfg), Options::default());

            single.scanners.insert("good".to_string(), good);
            single.scanners.insert("bad".to_string(), bad);
            single.deployed.insert("good".to_string());
            single.deployed.insert("bad".to_string());

            // advance past the window and run one tick. The bad collection's discover
            // errors, so the good collection still delivers.
            clock.advance(1);
            single.scan().await.unwrap();

            // the good collection's StableFile was delivered despite the
            // sibling error — and nothing else was ("only the good rule
            // delivers").
            let delivered = sink.events();
            assert_eq!(delivered.len(), 1, "only the good collection delivers");
            let sf = &delivered[0].0;
            assert_eq!(stable_name(sf), "good.mcap".to_string());
            assert_eq!(sf.file_rule_id, "r-good".to_string());
        }
    }

    mod prune {
        use super::*;

        // internal crates
        use crate::data_uploads::scan::rule::Options;
        use crate::data_uploads::scan::state::{Candidate, Observation};

        // external crates
        use std::time::SystemTime;

        const LEDGER_PRUNE_THRESHOLD: usize = 1000;

        /// A single-entry ledger history for `file` (fixed synthetic metadata).
        fn ledger_entry(file: &File) -> Vec<StableFile> {
            vec![StableFile {
                file: file.clone(),
                size: 4,
                digest: "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
                mtime: DateTime::from_timestamp(0, 0).unwrap(),
                mtime_aliases: vec![],
                first_observed_at: DateTime::from_timestamp(900, 0).unwrap(),
                last_observed_at: DateTime::from_timestamp(900, 0).unwrap(),
                deployment_id: "d".to_string(),
                file_rule_id: DEFAULT_RULE_ID.to_string(),
            }]
        }

        /// Seed `n` ledger histories keyed to never-created `gone{i}.mcap`
        /// paths inside `dir` (absent from every glob result).
        fn seed_stale(state: &mut RuleState, dir: &Dir, n: usize) -> Vec<File> {
            let mut files = Vec::with_capacity(n);
            for i in 0..n {
                let file = dir.file(&format!("gone{i}.mcap"));
                state.ledger.insert(file.clone(), ledger_entry(&file));
                files.push(file);
            }
            files
        }

        /// A RuleState for `DEFAULT_RULE_ID` globbing `dir` with a
        /// threshold-opening ledger: one entry for the (real) `live` file plus
        /// LEDGER_PRUNE_THRESHOLD stale entries. Returns the stale keys too.
        fn padded_state(dir: &Dir, live: &File, window: i64) -> (RuleState, Vec<File>) {
            let cfg = Config {
                deployment: deployment("d"),
                rule: upload_rule(DEFAULT_RULE_ID, &mcap_glob(dir), window),
            };
            let mut state = RuleState::new(cfg);
            state.ledger.insert(live.clone(), ledger_entry(live));
            let stale = seed_stale(&mut state, dir, LEDGER_PRUNE_THRESHOLD);
            (state, stale)
        }

        // A scan tick prunes a deployed collection's over-threshold ledger via
        // its discovery pass: glob-absent entries drop, the globbed file's
        // entry survives.
        #[tokio::test]
        async fn scan_prunes_ledger_via_discovery() {
            let clock = Clock::new(1000);
            let mut single = SingleThreadScanner::new(ScannerArgs {
                now_fn: Arc::new(clock.now_fn()),
                sinks: vec![Arc::new(RecordingSink::new())],
                snapshot_file: None,
            })
            .unwrap();

            let dir = dirs::temp("testing").unwrap();
            let live = write(&dir, "live.mcap", b"aaaa").await;
            let (state, stale) = padded_state(&dir, &live, 0);
            single.scanners.insert(
                DEFAULT_RULE_ID.to_string(),
                RuleScanner::from_state(
                    state,
                    Options {
                        prune_threshold: LEDGER_PRUNE_THRESHOLD,
                    },
                ),
            );
            single.deployed.insert(DEFAULT_RULE_ID.to_string());

            single.scan().await.unwrap();

            let ledger = &single.scanners.get(DEFAULT_RULE_ID).unwrap().state().ledger;
            assert_eq!(ledger.len(), 1);
            assert!(ledger.contains_key(&live));
            assert!(!ledger.contains_key(&stale[0]));
        }

        // The prune result rides the scan pass's existing persist: after one
        // tick the on-disk snapshot no longer holds the stale keys (there is
        // no dedicated prune-persist path anymore).
        #[tokio::test]
        async fn scan_prune_is_persisted() {
            let dir = dirs::temp("testing").unwrap();
            let state_path = dir.file("scanner.json");
            let live = write(&dir, "live.mcap", b"aaaa").await;
            let (coll_state, stale) = padded_state(&dir, &live, 0);
            let padded = ScannerSnapshot {
                rules: HashMap::from([(DEFAULT_RULE_ID.to_string(), coll_state)]),
                deployed: HashSet::from([DEFAULT_RULE_ID.to_string()]),
            };
            let mut snapshot_file = state_file(&state_path).await;
            snapshot_file.patch(padded).await.unwrap();

            let clock = Clock::new(1000);
            let (scanner, _h) = Scanner::spawn(
                64,
                ScannerArgs {
                    now_fn: Arc::new(clock.now_fn()),
                    sinks: Vec::new(),
                    snapshot_file: Some(snapshot_file),
                },
            )
            .unwrap();
            scan_once(&scanner).await;

            let snapshot = read_snapshot(&state_path).await;
            let state = snapshot.rules.get(DEFAULT_RULE_ID).unwrap();
            assert_eq!(state.ledger.len(), 1);
            assert!(state.ledger.contains_key(&live));
            assert!(!state.ledger.contains_key(&stale[0]));
        }

        // An undeployed collection is never glob-pruned: scan() runs discovery
        // (and thus the prune) only for deployed collections. This asymmetry
        // is deliberate — an undeployed collection just drains its candidates
        // and is then removed wholesale, a stronger prune than the glob-set one.
        #[tokio::test]
        async fn undeployed_collection_is_not_pruned() {
            let clock = Clock::new(1000);
            let mut single = SingleThreadScanner::new(ScannerArgs {
                now_fn: Arc::new(clock.now_fn()),
                sinks: vec![Arc::new(RecordingSink::new())],
                snapshot_file: None,
            })
            .unwrap();

            let dir = dirs::temp("testing").unwrap();
            let live = write(&dir, "live.mcap", b"aaaa").await;
            // a large stability window keeps the candidate (and therefore the
            // undeployed collection) alive across the tick.
            let (mut state, stale) = padded_state(&dir, &live, 10_000);
            let waiting = dir.file("waiting.mcap");
            state.candidates.insert(
                waiting.clone(),
                Candidate {
                    file: waiting.clone(),
                    first_obs: Observation {
                        file: waiting,
                        timestamp: DateTime::from_timestamp(1000, 0).unwrap(),
                        size: 4,
                        mtime: SystemTime::UNIX_EPOCH,
                        deployment_id: "d".to_string(),
                        file_rule_id: DEFAULT_RULE_ID.to_string(),
                    },
                },
            );
            single.scanners.insert(
                DEFAULT_RULE_ID.to_string(),
                RuleScanner::from_state(
                    state,
                    Options {
                        prune_threshold: LEDGER_PRUNE_THRESHOLD,
                    },
                ),
            );
            // deliberately NOT inserted into `deployed`.

            single.scan().await.unwrap();

            let ledger = &single.scanners.get(DEFAULT_RULE_ID).unwrap().state().ledger;
            assert_eq!(ledger.len(), LEDGER_PRUNE_THRESHOLD + 1);
            assert!(ledger.contains_key(&stale[0]));
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
            assert!(matches!(
                err,
                crate::data_uploads::scan::ScanErr::SendActorMessageErr(_)
            ));
        }
    }
}
