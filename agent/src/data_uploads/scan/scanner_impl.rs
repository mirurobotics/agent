// standard crates
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

// internal crates
use crate::data_uploads::scan::{
    errors::*,
    rule::{Options, RuleScanner},
    sink::StableFileSink,
    state::{Config, RuleState, ScanSnapshotFile, ScannerSnapshot, StableFile},
};
use crate::filesys::files;
use crate::models::{Deployment, FileRule, FileRuleID};
use crate::trace;

// external crates
use chrono::{DateTime, Utc};
use tracing::{debug, info, warn};

/// The IDs of `rules`, or an error if an ID repeats or a glob is invalid.
pub(crate) fn validate_rules(rules: &[FileRule]) -> Result<HashSet<FileRuleID>, ScanErr> {
    let mut ids = HashSet::new();
    for rule in rules {
        if !ids.insert(rule.id.clone()) {
            return Err(ScanErr::DuplicateFileRuleID(DuplicateFileRuleID {
                file_rule_id: rule.id.clone(),
                trace: trace!(),
            }));
        }
        files::validate_glob(&rule.source.glob)?;
    }
    Ok(ids)
}

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

/// Single-threaded scanner state and logic. [`Scanner`] runs it inside an
/// actor so callers can share it across tasks.
///
/// [`Scanner`]: crate::data_uploads::scan::Scanner
pub struct ScannerImpl {
    scanners: HashMap<FileRuleID, RuleScanner>,
    deployed: HashSet<FileRuleID>,
    now_fn: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
    sinks: Vec<Arc<dyn StableFileSink>>,
    snapshot_file: Option<ScanSnapshotFile>,
}

impl ScannerImpl {
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

    #[cfg(test)]
    pub(super) async fn get_rules(&self) -> Result<Vec<FileRule>, ScanErr> {
        let rules = self
            .scanners
            .values()
            .map(|c| c.rule().clone())
            .collect::<Vec<_>>();
        Ok(rules)
    }

    #[cfg(test)]
    pub(super) async fn get_ledger_count(&self) -> Result<usize, ScanErr> {
        let count = self
            .scanners
            .values()
            .map(RuleScanner::ledger_count)
            .sum::<usize>();
        Ok(count)
    }

    pub(super) async fn clear_rules(&mut self) -> Result<(), ScanErr> {
        self.deployed.clear();
        self.persist_snapshot().await;
        info!("scan: cleared all deployed file rules");
        Ok(())
    }

    pub(super) async fn update_rules(
        &mut self,
        deployment: Deployment,
        rules: Vec<FileRule>,
    ) -> Result<(), ScanErr> {
        // Reject an invalid rule set whole, before touching any scanner, and
        // deploy nothing. Running its valid rules, or keeping the previous
        // deployment's rules, would upload and delete under a policy nobody
        // approved.
        let deployed = match validate_rules(&rules) {
            Ok(deployed) => deployed,
            Err(err) => {
                self.clear_rules().await?;
                return Err(err);
            }
        };

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

    pub(super) async fn scan(&mut self) -> Result<(), ScanErr> {
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

        // Dispatch before persisting. A crash in between leaves them as
        // candidates, so the next process re-emits them. Persisting first
        // would record them as done without ever queueing them.
        let delivered = stable_files.len();
        self.dispatch_stable_files(stable_files).await;
        self.persist_snapshot().await;

        debug!(
            "scan: tick complete; {delivered} stable file(s) delivered to sinks, \
             {pruned} inactive rule(s) pruned"
        );

        Ok(())
    }
}

#[cfg(test)]
pub(super) mod tests {
    // standard crates
    use std::collections::{HashMap, HashSet};
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::sync::{Arc, Mutex};

    // internal crates
    use super::{ScannerArgs, ScannerImpl};
    use crate::data_uploads::scan::rule::RuleScanner;
    use crate::data_uploads::scan::sink::StableFileSink;
    use crate::data_uploads::scan::state::{
        Config, RuleState, ScanSnapshotFile, ScannerSnapshot, StableFile,
    };
    use crate::filesys::{dirs, files, Dir, File, PathExt, WriteOptions};
    use crate::models::{Deployment, DplActivity, FileRule, FileRuleSource, FileRuleUpload};
    use crate::test_utils::filesys::dirs as test_dirs;

    // external crates
    use chrono::{DateTime, Utc};

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
    pub(crate) const DEFAULT_RULE_ID: &str = "r";

    /// A Deployed deployment with the given id (release_id mirrors the id).
    pub(crate) fn deployment(id: &str) -> Deployment {
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
    pub(crate) fn upload_rule(rule_id: &str, glob: &str, stability_window_secs: i64) -> FileRule {
        rule_in_collection(
            rule_id,
            &format!("{rule_id}-coll"),
            glob,
            stability_window_secs,
        )
    }

    /// Build a FileRule with a pinned `upload_collection_id` (plus rule id/glob/window).
    pub(crate) fn rule_in_collection(
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
        pub(crate) fn new() -> Self {
            Self::default()
        }

        /// The recorded `(StableFile, FileRule)` pairs, in delivery order.
        pub(crate) fn events(&self) -> Vec<(StableFile, FileRule)> {
            self.events.lock().unwrap().clone()
        }

        /// The recorded stable files' names, in delivery order.
        pub(crate) fn names(&self) -> Vec<String> {
            self.events().iter().map(|(f, _)| stable_name(f)).collect()
        }

        /// The number of recorded deliveries.
        pub(crate) fn count(&self) -> usize {
            self.events.lock().unwrap().len()
        }

        /// Assert exactly one delivery so far, named `name`. The caller supplies
        /// the WHY exactly-one holds at its own site.
        pub(crate) fn assert_one_stable(&self, name: &str) {
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

    /// Write `bytes` to `dir/name` and return the file.
    pub(crate) async fn write(dir: &Dir, name: &str, bytes: &[u8]) -> File {
        let file = dir.file(name);
        files::write_bytes(&file, bytes, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();
        file
    }

    /// The file name of a StableFile's underlying File.
    pub(crate) fn stable_name(sf: &StableFile) -> String {
        sf.file
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string()
    }

    pub(crate) async fn state_file(file: &File) -> ScanSnapshotFile {
        ScanSnapshotFile::new_with_default(file.clone(), ScannerSnapshot::default())
            .await
            .unwrap()
    }

    pub(crate) async fn read_snapshot(file: &File) -> ScannerSnapshot {
        files::read_json(file).await.unwrap()
    }

    pub(crate) fn mcap_glob(dir: &Dir) -> String {
        format!("{}/*.mcap", dir.path().display())
    }

    mod construction {
        use super::*;

        #[tokio::test]
        async fn existing_state_file_restores_scanner() {
            let dir = test_dirs::temp("testing").unwrap();
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

            let scanner = ScannerImpl::new(ScannerArgs {
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
            let dir = test_dirs::temp("testing").unwrap();
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

            let loaded = ScannerImpl::load_snapshot(&Some(snapshot_file));

            assert_eq!(loaded, expected);
        }
    }

    mod persist_snapshot {
        use super::*;

        // internal crates
        use crate::data_uploads::scan::rule::Options;

        #[tokio::test]
        async fn writes_current_snapshot() {
            let dir = test_dirs::temp("testing").unwrap();
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
            let mut scanner = ScannerImpl::new(ScannerArgs {
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
            let dir = test_dirs::temp("testing").unwrap();
            let state_path = dir.file("scanner.json");
            let rule_state = RuleState::new(Config {
                deployment: deployment("d"),
                rule: upload_rule(DEFAULT_RULE_ID, "/none/*.mcap", 0),
            });
            let mut scanner = ScannerImpl::new(ScannerArgs {
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

    mod validate_rules_fn {
        use super::*;
        use crate::data_uploads::scan::{scanner_impl::validate_rules, ScanErr};

        #[test]
        fn returns_ids() {
            let rules = vec![
                upload_rule("a", "/logs/*.log", 0),
                upload_rule("b", "/data/**/*.mcap", 0),
            ];
            let ids = validate_rules(&rules).unwrap();
            assert_eq!(ids, HashSet::from(["a".to_string(), "b".to_string()]));
        }

        #[test]
        fn empty() {
            assert!(validate_rules(&[]).unwrap().is_empty());
        }

        #[test]
        fn duplicate_id() {
            let rules = vec![
                upload_rule("a", "/logs/*.log", 0),
                upload_rule("a", "/data/*.mcap", 0),
            ];
            let err = validate_rules(&rules).unwrap_err();
            assert!(matches!(err, ScanErr::DuplicateFileRuleID(ref e) if e.file_rule_id == "a"));
        }

        #[test]
        fn invalid_glob() {
            let rules = vec![
                upload_rule("a", "/logs/*.log", 0),
                upload_rule("b", "/logs/[ab.log", 0),
            ];
            let err = validate_rules(&rules).unwrap_err();
            assert!(matches!(
                err,
                ScanErr::FileSysErr(crate::filesys::FileSysErr::InvalidGlobErr(_))
            ));
        }
    }

    mod scan {
        use super::*;

        // internal crates
        use crate::data_uploads::scan::rule::Options;

        // A discovery error in one rule does not prevent a sibling rule from
        // emitting its stable file.
        #[tokio::test]
        async fn scan_isolates_bad_glob_rule_from_emitting_sibling() {
            let clock = Clock::new(1000);
            let sink = RecordingSink::new();
            let mut single = ScannerImpl::new(ScannerArgs {
                now_fn: Arc::new(clock.now_fn()),
                sinks: vec![Arc::new(sink.clone())],
                snapshot_file: None,
            })
            .unwrap();

            // --- good rule: a real file, discovered as a candidate at t=1000. ---
            let good_dir = test_dirs::temp("testing").unwrap();
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
            let mut single = ScannerImpl::new(ScannerArgs {
                now_fn: Arc::new(clock.now_fn()),
                sinks: vec![Arc::new(RecordingSink::new())],
                snapshot_file: None,
            })
            .unwrap();

            let dir = test_dirs::temp("testing").unwrap();
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
            let dir = test_dirs::temp("testing").unwrap();
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
            let mut scanner = ScannerImpl::new(ScannerArgs {
                now_fn: Arc::new(clock.now_fn()),
                sinks: Vec::new(),
                snapshot_file: Some(snapshot_file),
            })
            .unwrap();
            scanner.scan().await.unwrap();

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
            let mut single = ScannerImpl::new(ScannerArgs {
                now_fn: Arc::new(clock.now_fn()),
                sinks: vec![Arc::new(RecordingSink::new())],
                snapshot_file: None,
            })
            .unwrap();

            let dir = test_dirs::temp("testing").unwrap();
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
}
