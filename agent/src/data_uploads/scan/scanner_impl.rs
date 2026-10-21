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
    pub(super) scanners: HashMap<FileRuleID, RuleScanner>,
    pub(super) deployed: HashSet<FileRuleID>,
    now_fn: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
    sinks: Vec<Arc<dyn StableFileSink>>,
    pub(super) snapshot_file: Option<ScanSnapshotFile>,
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

    pub(super) fn load_snapshot(state_file: &Option<ScanSnapshotFile>) -> ScannerSnapshot {
        let Some(state_file) = state_file.as_ref() else {
            return ScannerSnapshot {
                rules: HashMap::new(),
                deployed: HashSet::new(),
            };
        };
        state_file.read().as_ref().clone()
    }

    pub(super) async fn persist_snapshot(&mut self) {
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
