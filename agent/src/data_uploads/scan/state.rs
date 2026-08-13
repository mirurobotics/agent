// standard crates
use std::collections::{HashMap, HashSet};

// internal crates
use crate::filesys::{state_file::SingleThreadStateFile, File};
use crate::models::{Deployment, FileRule, FileRuleID, Patch};

// external crates
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuleState {
    pub(crate) cfg: Config,
    pub(crate) preexisting: HashMap<File, Observation>,
    pub(crate) candidates: HashMap<File, Candidate>,
    pub(crate) ledger: HashMap<File, Vec<StableFile>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub deployment: Deployment,
    pub rule: FileRule,
}

impl RuleState {
    pub(crate) fn new(cfg: Config) -> Self {
        Self {
            cfg,
            preexisting: HashMap::new(),
            candidates: HashMap::new(),
            ledger: HashMap::new(),
        }
    }

    pub(crate) fn rule(&self) -> &FileRule {
        &self.cfg.rule
    }

    #[cfg(feature = "test")]
    pub(crate) fn ledger_count(&self) -> usize {
        self.ledger.len()
    }

    pub(crate) fn has_candidates(&self) -> bool {
        !self.candidates.is_empty()
    }

    pub(crate) fn is_candidate(&self, file: &File) -> bool {
        self.candidates.contains_key(file)
    }

    pub(crate) fn is_preexisting(&self, obs: &Observation) -> bool {
        self.preexisting
            .get(&obs.file)
            .is_some_and(|preexisting| preexisting.equal_metadata(obs))
    }

    pub(crate) fn is_latest_ledger_entry(&self, obs: &Observation) -> bool {
        self.ledger
            .get(&obs.file)
            .and_then(|entries| entries.last())
            .is_some_and(|latest| latest.equal_metadata(obs))
    }

    pub(crate) fn latest_ledger_entry_mut(&mut self, file: &File) -> Option<&mut StableFile> {
        self.ledger
            .get_mut(file)
            .and_then(|entries| entries.last_mut())
    }

    /// Drop ledger entries whose file is absent from this pass's glob set.
    pub(crate) fn prune_ledger(&mut self, retain: &[File], threshold: usize) {
        if self.ledger.len() < threshold {
            return;
        }
        let to_retain: HashSet<&File> = retain.iter().collect();
        self.ledger.retain(|file, _| to_retain.contains(file));
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observation {
    pub file: File,
    pub timestamp: DateTime<Utc>,
    pub size: u64,
    pub mtime: std::time::SystemTime,
    pub deployment_id: String,
    pub file_rule_id: String,
}

impl Observation {
    pub fn equal_metadata(&self, other: &Observation) -> bool {
        self.size == other.size && self.mtime == other.mtime
    }
}

// A file that is a candidate for upload. Holds the single observation taken at
// discovery; re-discovery of a tracked candidate is skipped (see
// `RuleState::is_candidate`), so a candidate never accumulates a second observation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub file: File,
    pub first_obs: Observation,
}

// A file that has been determined stable enough for upload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StableFile {
    pub file: File,
    pub size: u64,
    pub digest: String,
    pub mtime: DateTime<Utc>,
    pub mtime_aliases: Vec<DateTime<Utc>>,
    pub first_observed_at: DateTime<Utc>,
    pub last_observed_at: DateTime<Utc>,
    pub deployment_id: String,
    pub file_rule_id: String,
}

impl StableFile {
    pub fn equal_metadata(&self, other: &Observation) -> bool {
        self.size == other.size && self.has_mtime(other)
    }

    pub fn has_mtime(&self, other: &Observation) -> bool {
        let other_mtime = DateTime::<Utc>::from(other.mtime);
        self.mtime_aliases.contains(&other_mtime) || self.mtime == other_mtime
    }

    // adds the mtime alias to the stable file if it is not already present
    pub fn push_mtime_alias(&mut self, mtime: DateTime<Utc>) {
        if self.mtime == mtime || self.mtime_aliases.contains(&mtime) {
            return;
        }
        self.mtime_aliases.push(mtime);
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ScannerSnapshot {
    pub(crate) rules: HashMap<FileRuleID, RuleState>,
    pub(crate) deployed: HashSet<FileRuleID>,
}

impl Patch<ScannerSnapshot> for ScannerSnapshot {
    fn patch(&mut self, patch: ScannerSnapshot) {
        *self = patch;
    }
}

pub type ScanSnapshotFile = SingleThreadStateFile<ScannerSnapshot, ScannerSnapshot>;

#[cfg(test)]
mod tests {
    use super::*;

    // internal crates
    use crate::models::{FileRuleSource, FileRuleUpload};

    // external crates
    use std::time::SystemTime;

    /// SHA-256 of `b"aaaa"` — the digest `files::hash` returns for that content.
    const HASH_AAAA: &str =
        "sha256:61be55a8e2f6b4e172338bddf184d6dbee29c98853e0a0485ecee7f27b9af0b4";

    fn deployment(id: &str) -> Deployment {
        Deployment {
            id: id.to_string(),
            ..Default::default()
        }
    }

    fn rule(rule_id: &str, glob: &str, window: i64) -> FileRule {
        FileRule {
            id: rule_id.to_string(),
            upload: Some(FileRuleUpload {
                upload_collection_id: format!("{rule_id}-coll"),
                ..Default::default()
            }),
            source: FileRuleSource {
                glob: glob.to_string(),
                stability_window_secs: window,
            },
            ..Default::default()
        }
    }

    fn config(dpl_id: &str, rule_id: &str, glob: &str, window: i64) -> Config {
        Config {
            deployment: deployment(dpl_id),
            rule: rule(rule_id, glob, window),
        }
    }

    fn ts(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).unwrap()
    }

    fn stable_file(file: File, first_observed_at: DateTime<Utc>) -> StableFile {
        StableFile {
            file,
            size: 4,
            digest: HASH_AAAA.to_string(),
            mtime: ts(0),
            mtime_aliases: vec![],
            first_observed_at,
            last_observed_at: first_observed_at,
            deployment_id: "d".to_string(),
            file_rule_id: "r1".to_string(),
        }
    }

    /// A size-4 observation at ts(1000) with the epoch mtime for `file`.
    fn observation(file: File) -> Observation {
        Observation {
            file,
            timestamp: ts(1000),
            size: 4,
            mtime: SystemTime::UNIX_EPOCH,
            deployment_id: "d".to_string(),
            file_rule_id: "r1".to_string(),
        }
    }

    mod accessors {
        use super::*;

        #[test]
        fn new_starts_empty() {
            let state = RuleState::new(config("d", "r1", "/none/*.mcap", 0));

            assert_eq!(state.rule().id, "r1");
            assert!(state.preexisting.is_empty());
            assert!(state.candidates.is_empty());
            assert!(state.ledger.is_empty());
            assert_eq!(state.ledger_count(), 0);
            assert!(!state.has_candidates());
        }

        #[test]
        fn rule() {
            let mut cfg = config("d", "r1", "/logs/*.mcap", 60);
            cfg.rule.id = "rule-42".to_string();
            let state = RuleState::new(cfg);

            assert_eq!(state.rule().id, "rule-42");
            assert_eq!(state.rule().source.glob, "/logs/*.mcap");
        }

        #[test]
        fn ledger_count() {
            let mut state = RuleState::new(config("d", "r1", "/none/*.mcap", 0));
            assert_eq!(state.ledger_count(), 0);

            let file = File::new("/none/a.mcap");
            state
                .ledger
                .insert(file.clone(), vec![stable_file(file, ts(1000))]);
            assert_eq!(state.ledger_count(), 1);
        }

        #[test]
        fn has_candidates() {
            let mut state = RuleState::new(config("d", "r1", "/none/*.mcap", 0));
            assert!(!state.has_candidates());

            let file = File::new("/none/c.mcap");
            state.candidates.insert(
                file.clone(),
                Candidate {
                    file: file.clone(),
                    first_obs: observation(file.clone()),
                },
            );
            assert!(state.has_candidates());
        }

        #[test]
        fn is_candidate() {
            let file = File::new("/none/c.mcap");
            let mut state = RuleState::new(config("d", "r1", "/none/*.mcap", 0));

            assert!(!state.is_candidate(&file));

            state.candidates.insert(
                file.clone(),
                Candidate {
                    file: file.clone(),
                    first_obs: observation(file.clone()),
                },
            );
            assert!(state.is_candidate(&file));
        }
    }

    mod is_preexisting {
        use super::*;

        // external crates
        use std::time::Duration;

        fn observation(file: File) -> Observation {
            Observation {
                file,
                timestamp: ts(1000),
                size: 4,
                mtime: SystemTime::UNIX_EPOCH,
                deployment_id: "d".to_string(),
                file_rule_id: "r1".to_string(),
            }
        }

        #[test]
        fn does_not_exist() {
            let state = RuleState::new(config("d", "r1", "/none/*.mcap", 0));
            let obs = observation(File::new("/none/p.mcap"));
            assert!(!state.is_preexisting(&obs));
        }

        #[test]
        fn metadata_not_equal() {
            let file = File::new("/none/p.mcap");
            let mut state = RuleState::new(config("d", "r1", "/none/*.mcap", 0));
            state
                .preexisting
                .insert(file.clone(), observation(file.clone()));

            let mut size_changed = observation(file.clone());
            size_changed.size += 1;
            assert!(!state.is_preexisting(&size_changed));

            let mut mtime_changed = observation(file);
            mtime_changed.mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(1);
            assert!(!state.is_preexisting(&mtime_changed));
        }

        #[test]
        fn is_preexisting() {
            let file = File::new("/none/p.mcap");
            let mut state = RuleState::new(config("d", "r1", "/none/*.mcap", 0));
            let obs = observation(file.clone());
            state.preexisting.insert(file, obs.clone());
            assert!(state.is_preexisting(&obs));
        }
    }

    mod prune_ledger {
        use super::*;

        /// Seed `n` single-entry ledger histories keyed to `/none/{i}.mcap`,
        /// returning the seeded keys in order. No I/O: pruning never inspects
        /// the filesystem, only the glob set it is handed.
        fn seed_n(state: &mut RuleState, n: usize) -> Vec<File> {
            let mut files = Vec::with_capacity(n);
            for i in 0..n {
                let file = File::new(format!("/none/{i}.mcap"));
                state
                    .ledger
                    .insert(file.clone(), vec![stable_file(file.clone(), ts(1000))]);
                files.push(file);
            }
            files
        }

        // Below the threshold nothing is pruned, even against an empty glob
        // set (the maximally aggressive input): the audit-history guarantee.
        #[test]
        fn below_threshold_prunes_nothing() {
            let mut state = RuleState::new(config("d", "r1", "/none/*.mcap", 0));
            seed_n(&mut state, 2);

            state.prune_ledger(&[], 3);
            assert_eq!(state.ledger_count(), 2);
        }

        // Exactly at the threshold the gate opens (pins the >= boundary):
        // glob-set members keep their full Vec history, everything else drops.
        #[test]
        fn at_threshold_drops_unglobbed_keeps_globbed() {
            let mut state = RuleState::new(config("d", "r1", "/none/*.mcap", 0));
            let seeded = seed_n(&mut state, 4);
            let kept = &seeded[..3];
            // give one retained key a two-entry history to prove the whole
            // Vec survives, not just the latest entry.
            state
                .ledger
                .get_mut(&kept[0])
                .unwrap()
                .push(stable_file(kept[0].clone(), ts(2000)));

            state.prune_ledger(kept, 4);
            assert_eq!(state.ledger_count(), 3);
            for file in kept {
                assert!(state.ledger.contains_key(file));
            }
            assert_eq!(state.ledger.get(&kept[0]).unwrap().len(), 2);
            assert!(!state.ledger.contains_key(&seeded[3]));
        }

        // Pruning an empty ledger is a no-op (the gate returns early).
        #[test]
        fn empty_ledger_noop() {
            let mut state = RuleState::new(config("d", "r1", "/none/*.mcap", 0));
            state.prune_ledger(&[], 1);
            assert_eq!(state.ledger_count(), 0);
        }
    }

    mod observation {
        use super::*;

        // external crates
        use std::time::Duration;

        fn obs(file: File) -> Observation {
            Observation {
                file,
                timestamp: ts(1000),
                size: 4,
                mtime: SystemTime::UNIX_EPOCH,
                deployment_id: "d".to_string(),
                file_rule_id: "r1".to_string(),
            }
        }

        #[test]
        fn equal_metadata_equal() {
            let file = File::new("/none/a.mcap");
            let a = obs(file.clone());
            let b = obs(file);
            assert!(a.equal_metadata(&b));
        }

        #[test]
        fn equal_metadata_size_differs() {
            let file = File::new("/none/a.mcap");
            let a = obs(file.clone());
            let mut b = obs(file);
            b.size += 1;
            assert!(!a.equal_metadata(&b));
        }

        #[test]
        fn equal_metadata_mtime_differs() {
            let file = File::new("/none/a.mcap");
            let a = obs(file.clone());
            let mut b = obs(file);
            b.mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(1);
            assert!(!a.equal_metadata(&b));
        }
    }

    mod candidate {
        use super::*;

        #[test]
        fn holds_single_observation() {
            let file = File::new("/none/x.mcap");
            let obs = observation(file.clone());
            let cand = Candidate {
                file: file.clone(),
                first_obs: obs.clone(),
            };

            assert_eq!(cand.file, file);
            assert_eq!(cand.first_obs, obs);
        }
    }

    mod is_latest_ledger_entry {
        use super::*;

        // external crates
        use std::time::Duration;

        fn obs(file: File) -> Observation {
            Observation {
                file,
                timestamp: ts(1000),
                size: 4,
                mtime: SystemTime::UNIX_EPOCH,
                deployment_id: "d".to_string(),
                file_rule_id: "r1".to_string(),
            }
        }

        #[test]
        fn matches() {
            let file = File::new("/none/l.mcap");
            let mut state = RuleState::new(config("d", "r1", "/none/*.mcap", 0));
            let mut entry = stable_file(file.clone(), ts(900));
            entry.mtime = DateTime::<Utc>::from(SystemTime::UNIX_EPOCH);
            state.ledger.insert(file.clone(), vec![entry]);
            assert!(state.is_latest_ledger_entry(&obs(file)));
        }

        #[test]
        fn metadata_differs() {
            let file = File::new("/none/l.mcap");
            let mut state = RuleState::new(config("d", "r1", "/none/*.mcap", 0));
            let mut entry = stable_file(file.clone(), ts(900));
            entry.mtime = DateTime::<Utc>::from(SystemTime::UNIX_EPOCH);
            state.ledger.insert(file.clone(), vec![entry]);

            let mut size_changed = obs(file.clone());
            size_changed.size += 1;
            assert!(!state.is_latest_ledger_entry(&size_changed));

            let mut mtime_changed = obs(file);
            mtime_changed.mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(1);
            assert!(!state.is_latest_ledger_entry(&mtime_changed));
        }

        #[test]
        fn absent() {
            let file = File::new("/none/l.mcap");
            let state = RuleState::new(config("d", "r1", "/none/*.mcap", 0));
            assert!(!state.is_latest_ledger_entry(&obs(file)));
        }
    }

    mod latest_ledger_entry_mut {
        use super::*;

        #[test]
        fn appends_alias() {
            let file = File::new("/none/l.mcap");
            let mut state = RuleState::new(config("d", "r1", "/none/*.mcap", 0));
            state
                .ledger
                .insert(file.clone(), vec![stable_file(file.clone(), ts(900))]);

            state
                .latest_ledger_entry_mut(&file)
                .unwrap()
                .push_mtime_alias(ts(1234));
            let latest = state.ledger.get(&file).unwrap().last().unwrap();
            assert_eq!(latest.mtime_aliases, vec![ts(1234)]);
        }

        #[test]
        fn none_when_absent() {
            let file = File::new("/none/l.mcap");
            let mut state = RuleState::new(config("d", "r1", "/none/*.mcap", 0));
            assert!(state.latest_ledger_entry_mut(&file).is_none());
        }
    }

    mod push_mtime_alias {
        use super::*;

        #[test]
        fn dedups_against_primary_mtime() {
            let file = File::new("/none/l.mcap");
            let mut entry = stable_file(file, ts(900));
            entry.mtime = ts(500);
            entry.push_mtime_alias(ts(500));
            assert!(entry.mtime_aliases.is_empty());
        }

        #[test]
        fn dedups_repeated_alias() {
            let file = File::new("/none/l.mcap");
            let mut entry = stable_file(file, ts(900));
            entry.push_mtime_alias(ts(1234));
            entry.push_mtime_alias(ts(1234));
            entry.push_mtime_alias(ts(5678));
            entry.push_mtime_alias(ts(1234));
            assert_eq!(entry.mtime_aliases, vec![ts(1234), ts(5678)]);
        }
    }

    mod persistence {
        use super::*;

        /// A RuleState with one preexisting file, one candidate, and one ledger
        /// entry, so every map in the snapshot is exercised.
        fn populated_state() -> RuleState {
            let file = File::new("/none/p.mcap");
            let mut state = RuleState::new(config("d", "r1", "/none/*.mcap", 10));
            state
                .preexisting
                .insert(file.clone(), observation(file.clone()));
            state.candidates.insert(
                file.clone(),
                Candidate {
                    file: file.clone(),
                    first_obs: observation(file.clone()),
                },
            );
            state
                .ledger
                .insert(file.clone(), vec![stable_file(file, ts(1000))]);
            state
        }

        #[test]
        fn snapshot_round_trips_serde_json() {
            let original = ScannerSnapshot {
                rules: HashMap::from([("r1".to_string(), populated_state())]),
                deployed: HashSet::from(["r1".to_string()]),
            };
            let json = serde_json::to_string(&original).unwrap();
            let back: ScannerSnapshot = serde_json::from_str(&json).unwrap();
            assert_eq!(back, original);
        }

        #[test]
        fn patch_replaces_whole_value() {
            let mut old = ScannerSnapshot {
                rules: HashMap::from([("r1".to_string(), populated_state())]),
                deployed: HashSet::from(["r1".to_string()]),
            };
            let new = ScannerSnapshot {
                rules: HashMap::from([(
                    "r2".to_string(),
                    RuleState::new(config("d2", "r2", "/none/*.mcap", 0)),
                )]),
                deployed: HashSet::from(["r2".to_string()]),
            };
            old.patch(new.clone());
            assert_eq!(old, new);

            old.patch(ScannerSnapshot::default());
            assert_eq!(old, ScannerSnapshot::default());
        }
    }

    mod stable_file {
        use super::*;

        use std::time::Duration;

        fn obs(file: File) -> Observation {
            Observation {
                file,
                timestamp: ts(1000),
                size: 4,
                mtime: SystemTime::UNIX_EPOCH,
                deployment_id: "d".to_string(),
                file_rule_id: "r1".to_string(),
            }
        }

        #[test]
        fn matches_primary_mtime() {
            let file = File::new("/none/s.mcap");
            let mut entry = stable_file(file.clone(), ts(900));
            entry.mtime = DateTime::<Utc>::from(SystemTime::UNIX_EPOCH);
            let observation = obs(file);
            assert!(entry.has_mtime(&observation));
            assert!(entry.equal_metadata(&observation));
        }

        #[test]
        fn matches_aliased_mtime() {
            let file = File::new("/none/s.mcap");
            let aliased = SystemTime::UNIX_EPOCH + Duration::from_secs(7);
            let mut entry = stable_file(file.clone(), ts(900));
            entry.mtime = ts(0);
            entry.mtime_aliases = vec![DateTime::<Utc>::from(aliased)];
            let mut observation = obs(file);
            observation.mtime = aliased;
            assert!(entry.has_mtime(&observation));
            assert!(entry.equal_metadata(&observation));
        }

        #[test]
        fn size_mismatch() {
            let file = File::new("/none/s.mcap");
            let mut entry = stable_file(file.clone(), ts(900));
            entry.mtime = DateTime::<Utc>::from(SystemTime::UNIX_EPOCH);
            let mut observation = obs(file);
            observation.size += 1;
            assert!(!entry.equal_metadata(&observation));
        }

        #[test]
        fn mtime_mismatch() {
            let file = File::new("/none/s.mcap");
            let mut entry = stable_file(file.clone(), ts(900));
            entry.mtime = ts(0);
            let mut observation = obs(file);
            observation.mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(1);
            assert!(!entry.has_mtime(&observation));
            assert!(!entry.equal_metadata(&observation));
        }
    }
}
