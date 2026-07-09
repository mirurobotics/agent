// standard crates
use std::collections::HashMap;

// internal crates
use crate::filesys::File;
use crate::models::{Deployment, UploadRule};
use crate::scan::errors::*;
use crate::trace;

// external crates
use chrono::{DateTime, Utc};

#[derive(Debug, PartialEq)]
pub struct State {
    pub(crate) cfg: Config,
    pub(crate) preexisting: HashMap<File, Observation>,
    pub(crate) candidates: HashMap<File, Candidate>,
    pub(crate) ledger: HashMap<File, Vec<StableFile>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub deployment: Deployment,
    pub rule: UploadRule,
}

impl State {
    pub(crate) fn new(cfg: Config) -> Self {
        Self {
            cfg,
            preexisting: HashMap::new(),
            candidates: HashMap::new(),
            ledger: HashMap::new(),
        }
    }

    pub(crate) fn rule(&self) -> &UploadRule {
        &self.cfg.rule
    }

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
        let preexisting = if let Some(preexisting) = self.preexisting.get(&obs.file) {
            preexisting
        } else {
            return false;
        };
        preexisting.equal_metadata(obs)
    }

    pub(crate) fn is_latest_ledger_entry(&self, obs: &Observation) -> bool {
        let stable_files = if let Some(ledger) = self.ledger.get(&obs.file) {
            ledger
        } else {
            return false;
        };
        let latest = if let Some(latest) = stable_files.last() {
            latest
        } else {
            return false;
        };
        latest.equal_metadata(obs)
    }

    pub(crate) fn add_mtime_alias_to_latest_ledger_entry(
        &mut self,
        file: &File,
        mtime: DateTime<Utc>,
    ) -> Result<(), ScanErr> {
        let stable_files = if let Some(stable_files) = self.ledger.get_mut(file) {
            stable_files
        } else {
            return Err(ScanErr::InternalError(InternalError {
                message: "File not found in ledger".to_string(),
                trace: trace!(),
            }));
        };
        let last = if let Some(last) = stable_files.last_mut() {
            last
        } else {
            return Err(ScanErr::InternalError(InternalError {
                message: "No last entry found in ledger".to_string(),
                trace: trace!(),
            }));
        };
        last.mtime_aliases.push(mtime);
        Ok(())
    }

    pub(crate) fn set_config(&mut self, cfg: Config) -> Result<(), ScanErr> {
        if self.cfg.rule.upload_collection_id != cfg.rule.upload_collection_id {
            return Err(ScanErr::InvalidRule(InvalidRule {
                existing_upload_collection_id: self.cfg.rule.upload_collection_id.clone(),
                replacement_upload_collection_id: cfg.rule.upload_collection_id,
                trace: trace!(),
            }));
        }

        self.cfg = cfg;
        Ok(())
    }

    pub(crate) fn prune_ledger(&mut self, before: DateTime<Utc>) -> Result<(), ScanErr> {
        self.ledger.retain(|_, stable_files| {
            stable_files
                .last()
                .is_none_or(|stable_file| stable_file.first_observed_at >= before)
        });
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observation {
    pub file: File,
    pub timestamp: DateTime<Utc>,
    pub size: u64,
    pub mtime: std::time::SystemTime,
    pub deployment_id: String,
    pub upload_rule_id: String,
}

impl Observation {
    pub fn equal_metadata(&self, other: &Observation) -> bool {
        self.size == other.size && self.mtime == other.mtime
    }
}

// A file that is a candidate for upload.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub file: File,
    pub observations: Vec<Observation>,
}

impl Candidate {
    pub fn latest_observation(&self) -> Result<Observation, ScanErr> {
        self.observations
            .last()
            .cloned()
            .ok_or(ScanErr::InternalError(InternalError {
                message: "No observations found".to_string(),
                trace: trace!(),
            }))
    }

    pub fn first_observation(&self) -> Result<Observation, ScanErr> {
        self.observations
            .first()
            .cloned()
            .ok_or(ScanErr::InternalError(InternalError {
                message: "No observations found".to_string(),
                trace: trace!(),
            }))
    }
}

// A file that has been determined stable enough for upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StableFile {
    pub file: File,
    pub size: u64,
    pub digest: String,
    pub mtime: DateTime<Utc>,
    pub mtime_aliases: Vec<DateTime<Utc>>,
    pub first_observed_at: DateTime<Utc>,
    pub last_observed_at: DateTime<Utc>,
    pub deployment_id: String,
    pub upload_rule_id: String,
}

impl StableFile {
    pub fn equal_metadata(&self, other: &Observation) -> bool {
        self.size == other.size && self.has_mtime(other)
    }

    pub fn has_mtime(&self, other: &Observation) -> bool {
        let other_mtime = DateTime::<Utc>::from(other.mtime);
        self.mtime_aliases.contains(&other_mtime) || self.mtime == other_mtime
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // internal crates
    use crate::models::UploadRuleSource;

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

    fn rule(collection_id: &str, glob: &str, window: i32) -> UploadRule {
        UploadRule {
            upload_collection_id: collection_id.to_string(),
            source: UploadRuleSource {
                glob: glob.to_string(),
                stability_window_secs: window,
            },
            ..Default::default()
        }
    }

    fn config(dpl_id: &str, collection_id: &str, glob: &str, window: i32) -> Config {
        Config {
            deployment: deployment(dpl_id),
            rule: rule(collection_id, glob, window),
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
            upload_rule_id: "coll".to_string(),
        }
    }

    mod accessors {
        use super::*;

        #[test]
        fn new_starts_empty() {
            let state = State::new(config("d", "coll", "/none/*.mcap", 0));

            assert_eq!(state.rule().upload_collection_id, "coll");
            assert!(state.preexisting.is_empty());
            assert!(state.candidates.is_empty());
            assert!(state.ledger.is_empty());
            assert_eq!(state.ledger_count(), 0);
            assert!(!state.has_candidates());
        }

        #[test]
        fn rule() {
            let mut cfg = config("d", "coll", "/logs/*.mcap", 60);
            cfg.rule.id = "rule-42".to_string();
            let state = State::new(cfg);

            assert_eq!(state.rule().id, "rule-42");
            assert_eq!(state.rule().source.glob, "/logs/*.mcap");
        }

        #[test]
        fn ledger_count() {
            let mut state = State::new(config("d", "coll", "/none/*.mcap", 0));
            assert_eq!(state.ledger_count(), 0);

            let file = File::new("/none/a.mcap");
            state
                .ledger
                .insert(file.clone(), vec![stable_file(file, ts(1000))]);
            assert_eq!(state.ledger_count(), 1);
        }

        #[test]
        fn has_candidates() {
            let mut state = State::new(config("d", "coll", "/none/*.mcap", 0));
            assert!(!state.has_candidates());

            let file = File::new("/none/c.mcap");
            state.candidates.insert(
                file.clone(),
                Candidate {
                    file: file.clone(),
                    observations: vec![],
                },
            );
            assert!(state.has_candidates());
        }

        #[test]
        fn is_candidate() {
            let file = File::new("/none/c.mcap");
            let mut state = State::new(config("d", "coll", "/none/*.mcap", 0));

            assert!(!state.is_candidate(&file));

            let obs = Observation {
                file: file.clone(),
                timestamp: ts(1000),
                size: 4,
                mtime: SystemTime::UNIX_EPOCH,
                deployment_id: "d".to_string(),
                upload_rule_id: "coll".to_string(),
            };
            state.candidates.insert(
                file.clone(),
                Candidate {
                    file: file.clone(),
                    observations: vec![obs],
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
                upload_rule_id: "coll".to_string(),
            }
        }

        #[test]
        fn does_not_exist() {
            let state = State::new(config("d", "coll", "/none/*.mcap", 0));
            let obs = observation(File::new("/none/p.mcap"));
            assert!(!state.is_preexisting(&obs));
        }

        #[test]
        fn metadata_not_equal() {
            let file = File::new("/none/p.mcap");
            let mut state = State::new(config("d", "coll", "/none/*.mcap", 0));
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
            let mut state = State::new(config("d", "coll", "/none/*.mcap", 0));
            let obs = observation(file.clone());
            state.preexisting.insert(file, obs.clone());
            assert!(state.is_preexisting(&obs));
        }
    }

    mod set_config {
        use super::*;

        #[test]
        fn collect_ids_differ() {
            let coll_id_a = "coll_a".to_string();
            let coll_id_b = "coll_b".to_string();
            let mut state = State::new(config("d", &coll_id_a, "/none/*.mcap", 0));
            let err = state
                .set_config(config("d", &coll_id_b, "/none/*.mcap", 0))
                .unwrap_err();
            assert!(matches!(err, ScanErr::InvalidRule(_)));
            assert_eq!(state.rule().upload_collection_id, "coll_a".to_string());
        }

        #[test]
        fn success() {
            let mut state = State::new(config("d", "coll", "/old/*.mcap", 0));
            let mut replacement = rule("coll", "/new/*.mcap", 5);
            replacement.id = "r2".to_string();
            state
                .set_config(Config {
                    deployment: deployment("d"),
                    rule: replacement,
                })
                .unwrap();
            assert_eq!(state.rule().id, "r2".to_string());
            assert_eq!(state.rule().source.glob, "/new/*.mcap".to_string());
        }
    }

    mod prune_ledger {
        use super::*;

        #[test]
        fn retains_when_last_entry_after_cutoff() {
            let file = File::new("/none/p.mcap");
            let mut state = State::new(config("d", "coll", "/none/*.mcap", 0));
            state.ledger.insert(
                file.clone(),
                vec![
                    stable_file(file.clone(), ts(900)),
                    stable_file(file.clone(), ts(1100)),
                ],
            );

            state.prune_ledger(ts(1000)).unwrap();
            assert_eq!(state.ledger_count(), 1);
            assert_eq!(state.ledger.get(&file).unwrap().len(), 2);
        }

        #[test]
        fn drops_when_last_entry_before_cutoff() {
            let file = File::new("/none/p.mcap");
            let mut state = State::new(config("d", "coll", "/none/*.mcap", 0));
            state.ledger.insert(
                file.clone(),
                vec![
                    stable_file(file.clone(), ts(1100)),
                    stable_file(file.clone(), ts(900)),
                ],
            );

            state.prune_ledger(ts(1000)).unwrap();
            assert_eq!(state.ledger_count(), 0);
        }

        #[test]
        fn prune_last_strictly_before_drops() {
            let file = File::new("/none/p.mcap");
            let mut state = State::new(config("d", "coll", "/none/*.mcap", 0));
            state
                .ledger
                .insert(file.clone(), vec![stable_file(file, ts(1000))]);
            state.prune_ledger(ts(1001)).unwrap();
            assert_eq!(state.ledger_count(), 0);
        }

        #[test]
        fn prune_last_strictly_after_retains() {
            let file = File::new("/none/p.mcap");
            let mut state = State::new(config("d", "coll", "/none/*.mcap", 0));
            state
                .ledger
                .insert(file.clone(), vec![stable_file(file, ts(1000))]);
            state.prune_ledger(ts(999)).unwrap();
            assert_eq!(state.ledger_count(), 1);
        }

        #[test]
        fn prune_last_exact_equality_retains() {
            let file = File::new("/none/p.mcap");
            let mut state = State::new(config("d", "coll", "/none/*.mcap", 0));
            state
                .ledger
                .insert(file.clone(), vec![stable_file(file, ts(1000))]);
            state.prune_ledger(ts(1000)).unwrap();
            assert_eq!(state.ledger_count(), 1);
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
                upload_rule_id: "coll".to_string(),
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
        fn first_and_latest_observation() {
            let file = File::new("/none/x.mcap");
            let first = Observation {
                file: file.clone(),
                timestamp: ts(1000),
                size: 4,
                mtime: SystemTime::UNIX_EPOCH,
                deployment_id: "d".to_string(),
                upload_rule_id: "coll".to_string(),
            };
            let latest = Observation {
                file: file.clone(),
                timestamp: ts(2000),
                size: 4,
                mtime: SystemTime::UNIX_EPOCH,
                deployment_id: "d".to_string(),
                upload_rule_id: "coll".to_string(),
            };
            let cand = Candidate {
                file,
                observations: vec![first.clone(), latest.clone()],
            };

            assert_eq!(cand.first_observation().unwrap().timestamp, first.timestamp);
            assert_eq!(
                cand.latest_observation().unwrap().timestamp,
                latest.timestamp
            );
        }

        #[test]
        fn empty_observations_error() {
            let cand = Candidate {
                file: File::new("/none/x.mcap"),
                observations: vec![],
            };
            assert!(matches!(
                cand.latest_observation(),
                Err(ScanErr::InternalError(_))
            ));
            assert!(matches!(
                cand.first_observation(),
                Err(ScanErr::InternalError(_))
            ));
        }
    }
}
