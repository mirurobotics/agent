// internal crates
pub use crate::data_uploads::scan::state::{Config, StableFile};
use crate::data_uploads::scan::{
    errors::*,
    scanner_impl::{ScannerArgs, ScannerImpl},
};
use crate::models::{Deployment, FileRule};
use crate::trace;

// external crates
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
    #[cfg(test)]
    GetRules {
        respond_to: oneshot::Sender<Result<Vec<FileRule>, ScanErr>>,
    },
    #[cfg(test)]
    GetLedgerCount {
        respond_to: oneshot::Sender<Result<usize, ScanErr>>,
    },
}

pub struct Worker {
    scanner: ScannerImpl,
    receiver: mpsc::Receiver<Command>,
}

impl Worker {
    pub fn new(scanner: ScannerImpl, receiver: mpsc::Receiver<Command>) -> Self {
        Self { scanner, receiver }
    }

    // lint:allow(funclen) — actor dispatch table; one arm per Command variant
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
                #[cfg(test)]
                Command::GetRules { respond_to } => {
                    dispatch!(
                        self.scanner.get_rules().await,
                        respond_to,
                        "Actor failed to send get rules response"
                    );
                }
                #[cfg(test)]
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

/// Command handle to the [`ScannerImpl`] actor. Reactive, not
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
            scanner: ScannerImpl::new(args)?,
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

    #[cfg(test)]
    pub(crate) async fn get_rules(&self) -> Result<Vec<FileRule>, ScanErr> {
        self.send_command(|tx| Command::GetRules { respond_to: tx })
            .await?
    }

    #[cfg(test)]
    pub(crate) async fn get_ledger_count(&self) -> Result<usize, ScanErr> {
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
    use std::collections::BTreeSet;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};

    // internal crates
    use super::ScannerExt;
    use super::{Scanner, ScannerArgs, ScannerImpl, StableFile, Worker};
    use crate::data_uploads::scan::scanner_impl::tests::{
        deployment, mcap_glob, read_snapshot, rule_in_collection, stable_name, state_file,
        upload_rule, write, Clock, RecordingSink, DEFAULT_RULE_ID,
    };
    use crate::data_uploads::scan::sink::StableFileSink;
    use crate::data_uploads::scan::state::ScannerSnapshot;
    use crate::filesys::{files, File, PathExt};
    use crate::models::FileRule;
    use crate::test_utils::filesys::dirs as test_dirs;
    use crate::test_utils::filesys::files as test_files;

    // external crates
    use chrono::{DateTime, Utc};
    use tokio::sync::mpsc;

    // =============================== TEST HELPERS ================================= //

    /// A retention-only FileRule: scanned and ledgered, but never uploaded.
    fn retention_only_rule(rule_id: &str, glob: &str, stability_window_secs: i64) -> FileRule {
        FileRule {
            upload: None,
            ..upload_rule(rule_id, glob, stability_window_secs)
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
    async fn single_rule(window: i64) -> (test_dirs::TempDir, Clock, Scanner) {
        let dir = test_dirs::temp("testing").unwrap();
        let glob = format!("{}/*.mcap", dir.path().display());
        let clock = Clock::new(1000);
        let scanner = spawn_scanner(&clock);
        deploy_single_rule(&scanner, &glob, window).await;
        (dir, clock, scanner)
    }

    /// [`single_rule`], but with a recording sink attached at spawn.
    async fn single_rule_with_sink(
        window: i64,
    ) -> (test_dirs::TempDir, Clock, Scanner, RecordingSink) {
        let dir = test_dirs::temp("testing").unwrap();
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

    struct PersistedScannerFixture {
        dir: test_dirs::TempDir,
        clock: Clock,
        scanner: Scanner,
        state_path: File,
    }

    async fn persisted_rule(window: i64) -> PersistedScannerFixture {
        let dir = test_dirs::temp("testing").unwrap();
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
            let single = ScannerImpl::new(ScannerArgs::default()).unwrap();
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
            let dir = test_dirs::temp("testing").unwrap();
            let state_path = dir.file("scanner.json");
            test_files::seed(&state_path, "not json").await;

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
            let dir = test_dirs::temp("testing").unwrap();
            let state_path = dir.file("scanner.json");
            test_files::seed(
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
    }

    mod sinks {
        use super::*;

        // a delivered StableFile carries the expected payload; dispatch is
        // awaited inside scan(), so the sink is populated when the tick returns.
        #[tokio::test]
        async fn sink_receives_stable_file_payload() {
            let dir = test_dirs::temp("testing").unwrap();
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

        /// A [`StableFileSink`] that, at delivery time, reads the scanner
        /// snapshot back off disk and records how many ledger entries it
        /// already held. Pins the dispatch-before-persist order.
        struct SnapshotProbeSink {
            state_path: File,
            seen: Arc<Mutex<Vec<usize>>>,
        }

        impl StableFileSink for SnapshotProbeSink {
            fn on_stable_file<'a>(
                &'a self,
                _file: StableFile,
                _rule: &'a FileRule,
            ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
                Box::pin(async move {
                    let snapshot = read_snapshot(&self.state_path).await;
                    let ledgered = snapshot.rules.values().map(|r| r.ledger_count()).sum();
                    self.seen.lock().unwrap().push(ledgered);
                })
            }
        }

        // The ledger that marks a file as reported reaches disk only after the
        // sinks have taken the file. If the agent died mid-dispatch, the
        // on-disk snapshot would still hold the file as a candidate and the
        // next process would re-emit it instead of losing it.
        #[tokio::test]
        async fn ledger_is_persisted_after_dispatch() {
            let dir = test_dirs::temp("testing").unwrap();
            let state_path = dir.file("scanner.json");
            let clock = Clock::new(1000);
            let seen = Arc::new(Mutex::new(Vec::new()));
            let probe = SnapshotProbeSink {
                state_path: state_path.clone(),
                seen: seen.clone(),
            };
            let (scanner, _h) = Scanner::spawn(
                64,
                ScannerArgs {
                    now_fn: Arc::new(clock.now_fn()),
                    sinks: vec![Arc::new(probe)],
                    snapshot_file: Some(state_file(&state_path).await),
                },
            )
            .unwrap();
            deploy(
                &scanner,
                vec![upload_rule(DEFAULT_RULE_ID, &mcap_glob(&dir), 0)],
            )
            .await;

            write(&dir, "a.mcap", b"aaa").await;
            scan_once(&scanner).await; // discover
            tick(&scanner, &clock, 1).await; // evaluate => deliver

            // during delivery the on-disk ledger did not yet record the file...
            assert_eq!(*seen.lock().unwrap(), vec![0]);
            // ...and once the tick finished, it did.
            let snapshot = read_snapshot(&state_path).await;
            let ledgered: usize = snapshot.rules.values().map(|r| r.ledger_count()).sum();
            assert_eq!(ledgered, 1);
        }

        /// A retention-only rule runs the full scan pipeline — glob, stability
        /// window, ledger, deliver — and its delivery carries `upload: None` so
        /// sinks know not to mint an upload job.
        #[tokio::test]
        async fn retention_only_rule_scans_and_delivers_without_upload() {
            let dir = test_dirs::temp("testing").unwrap();
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
            let dir = test_dirs::temp("testing").unwrap();
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
            let dir = test_dirs::temp("testing").unwrap();
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

        // A rule set with two rules sharing one rule id is rejected whole, and
        // the previous rules leave the deployed set.
        #[tokio::test]
        async fn update_rules_duplicate_rule_id_deploys_nothing() {
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

            // No scanner was created for the duplicate set, and the previous
            // rule drains on the next tick.
            assert_eq!(active_rule_ids(&scanner).await, before);
            scan_once(&scanner).await;
            assert_eq!(active_rule_ids(&scanner).await, BTreeSet::new());
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
            let dir = test_dirs::temp("testing").unwrap();
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
            let dir = test_dirs::temp("testing").unwrap();
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
            let dir = test_dirs::temp("testing").unwrap();
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

        // A rule set with one malformed glob is rejected whole: no rule from
        // it deploys, and the previous rules leave the deployed set and drain.
        #[tokio::test]
        async fn update_rules_invalid_glob_deploys_nothing() {
            let dir = test_dirs::temp("testing").unwrap();
            let glob = mcap_glob(&dir);
            let clock = Clock::new(1000);
            let (scanner, sink) = spawn_scanner_with_sink(&clock);

            deploy(&scanner, vec![upload_rule("old", &glob, 0)]).await;

            let rules = vec![
                upload_rule("new", &glob, 0),
                upload_rule("bad", "/logs/[ab.log", 0),
            ];
            let err = scanner
                .update_rules(deployment("d2"), rules)
                .await
                .unwrap_err();
            assert!(matches!(
                err,
                crate::data_uploads::scan::ScanErr::FileSysErr(
                    crate::filesys::FileSysErr::InvalidGlobErr(_)
                )
            ));

            scan_once(&scanner).await;
            assert_eq!(active_rule_ids(&scanner).await, BTreeSet::new());

            write(&dir, "a.mcap", b"aaa").await;
            scan_once(&scanner).await;
            tick(&scanner, &clock, 1).await;
            assert_eq!(sink.events(), vec![]);
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
            let dir = test_dirs::temp("testing").unwrap();
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
