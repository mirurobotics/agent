// standard crates
use std::collections::HashMap;

// internal crates
use crate::filesys::{errors::*, files, File, PathExt};
use crate::models::{Deployment, UploadRule};
use crate::scan::errors::*;
use crate::trace;

// external crates
use chrono::{DateTime, Utc};

pub struct State {
    cfg: Config,
    preexisting: HashMap<File, Observation>,
    candidates: HashMap<File, Candidate>,
    ledger: HashMap<File, Vec<StableFile>>,
}

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

    fn rule(&self) -> &UploadRule {
        &self.cfg.rule
    }

    fn ledger_count(&self) -> usize {
        self.ledger.len()
    }

    fn has_candidates(&self) -> bool {
        !self.candidates.is_empty()
    }

    fn is_candidate(&self, file: &File) -> bool {
        self.candidates.contains_key(file)
    }

    fn is_preexisting(&self, obs: &Observation) -> bool {
        let preexisting = if let Some(preexisting) = self.preexisting.get(&obs.file) {
            preexisting
        } else {
            return false;
        };
        preexisting.equal_metadata(obs)
    }

    fn set_config(&mut self, cfg: Config) -> Result<(), ScanErr> {
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

    fn prune_ledger(&mut self, before: DateTime<Utc>) -> Result<(), ScanErr> {
        self.ledger.retain(|_, stable_files| {
            stable_files
                .last()
                .is_none_or(|stable_file| stable_file.first_observed_at >= before)
        });
        Ok(())
    }
}

#[derive(Clone)]
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
#[derive(Clone)]
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
    pub first_observed_at: DateTime<Utc>,
    pub last_observed_at: DateTime<Utc>,
    pub deployment_id: String,
    pub upload_rule_id: String,
}

/// Owned (non-actor) sub-scanner for a single upload collection.
pub(crate) struct CollectionScanner {
    state: State,
}

impl CollectionScanner {
    pub(crate) async fn new(config: Config, now: DateTime<Utc>) -> Result<Self, ScanErr> {
        let mut state = State::new(config);
        state.preexisting = discover_preexisting(&state, now).await?;
        Ok(Self::from_state(state))
    }

    pub(crate) fn from_state(state: State) -> Self {
        Self { state }
    }

    pub(crate) fn rule(&self) -> &UploadRule {
        self.state.rule()
    }

    pub(crate) fn ledger_count(&self) -> usize {
        self.state.ledger_count()
    }

    pub(crate) fn has_candidates(&self) -> bool {
        self.state.has_candidates()
    }

    /// Replace only the active rule, carrying over observation/dedupe/cadence state.
    pub(crate) async fn update_config(
        &mut self,
        config: Config,
        now: DateTime<Utc>,
    ) -> Result<(), ScanErr> {
        self.state.set_config(config)?;

        // rediscover preexisting files
        let preexisting = discover_preexisting(&self.state, now).await?;
        self.state.preexisting = preexisting;

        Ok(())
    }

    pub(crate) async fn evaluate_candidates(
        &mut self,
        now: DateTime<Utc>,
    ) -> Result<Vec<StableFile>, ScanErr> {
        let mut stable_files = Vec::new();
        let candidates = self.state.candidates.clone();
        for candidate in candidates.values() {
            let action = eval_candidate(&self.state, candidate, now).await?;
            match action {
                EvalAction::WaitForStabilityWindow => continue,
                EvalAction::Unstable => {
                    self.state.candidates.remove(&candidate.file);
                }
                EvalAction::Stable(stable_file) => {
                    stable_files.push(stable_file.clone());
                    self.state
                        .ledger
                        .insert(candidate.file.clone(), vec![stable_file]);
                }
            }
        }
        Ok(stable_files)
    }

    pub(crate) async fn discover_candidates(
        &mut self,
        now: DateTime<Utc>,
    ) -> Result<Vec<Candidate>, ScanErr> {
        let candidates = discover_candidates(&self.state, now).await?;
        for candidate in candidates.iter() {
            self.state
                .candidates
                .insert(candidate.file.clone(), candidate.clone());
        }
        Ok(candidates)
    }

    pub(crate) fn prune_ledger(&mut self, before: DateTime<Utc>) -> Result<(), ScanErr> {
        self.state.prune_ledger(before)
    }
}

pub async fn discover_preexisting(
    state: &State,
    now: DateTime<Utc>,
) -> Result<HashMap<File, Observation>, ScanErr> {
    let mut preexisting = HashMap::new();
    for file in files::glob(&state.cfg.rule.source.glob)? {
        if state.is_candidate(&file) {
            continue;
        }
        let observation = observe_file(state, file.clone(), now).await?;
        preexisting.insert(file, observation);
    }
    Ok(preexisting)
}

pub async fn discover_candidates(
    state: &State,
    now: DateTime<Utc>,
) -> Result<Vec<Candidate>, ScanErr> {
    let mut candidates = Vec::new();
    for file in files::glob(&state.cfg.rule.source.glob)? {
        if state.is_candidate(&file) {
            continue;
        }
        let observation = observe_file(state, file.clone(), now).await?;
        if state.is_preexisting(&observation) {
            continue;
        }
        candidates.push(Candidate {
            file,
            observations: vec![observation],
        });
    }
    Ok(candidates)
}

pub async fn observe_file(
    state: &State,
    file: File,
    timestamp: DateTime<Utc>,
) -> Result<Observation, ScanErr> {
    let meta = files::metadata(&file).await?;
    let mtime = meta.modified().map_err(|source| {
        ScanErr::FileSysErr(FileSysErr::FileMetadataErr(FileMetadataErr {
            file: file.clone(),
            source: Box::new(source),
            trace: trace!(),
        }))
    })?;
    Ok(Observation {
        file,
        timestamp,
        size: meta.len(),
        mtime,
        deployment_id: state.cfg.deployment.id.clone(),
        upload_rule_id: state.cfg.rule.upload_collection_id.clone(),
    })
}

pub enum EvalAction {
    WaitForStabilityWindow,
    Unstable,
    Stable(StableFile),
}

async fn eval_candidate(
    state: &State,
    candidate: &Candidate,
    now: DateTime<Utc>,
) -> Result<EvalAction, ScanErr> {
    if !has_stability_window_elapsed(state, candidate, now)? {
        return Ok(EvalAction::WaitForStabilityWindow);
    }

    if !candidate.file.exists() {
        // file was deleted
        return Ok(EvalAction::Unstable);
    }

    let observation = observe_file(state, candidate.file.clone(), now).await?;
    let outcome = determine_stability(state, candidate, &observation).await?;

    if outcome.is_stable {
        let first = candidate.first_observation()?;
        let last = candidate.latest_observation()?;
        debug_assert!(first.equal_metadata(&last));
        Ok(EvalAction::Stable(StableFile {
            file: candidate.file.clone(),
            size: observation.size,
            // A first-stable file (no same-size predecessor in the ledger) has
            // no recomputed digest; that is the empty string, not an error.
            digest: outcome.digest.clone().unwrap_or_default(),
            mtime: last.mtime.into(),
            first_observed_at: first.timestamp,
            last_observed_at: last.timestamp,
            deployment_id: first.deployment_id.clone(),
            upload_rule_id: first.upload_rule_id.clone(),
        }))
    } else {
        Ok(EvalAction::Unstable)
    }
}

fn has_stability_window_elapsed(
    state: &State,
    candidate: &Candidate,
    now: DateTime<Utc>,
) -> Result<bool, ScanErr> {
    let window = state.cfg.rule.source.stability_window_secs as i64;
    let stable_since = candidate.first_observation()?.timestamp;
    Ok(now.signed_duration_since(stable_since).num_seconds() >= window)
}

pub struct StabilityOutcome {
    pub is_stable: bool,
    pub digest: Option<String>,
}

async fn determine_stability(
    state: &State,
    candidate: &Candidate,
    observation: &Observation,
) -> Result<StabilityOutcome, ScanErr> {
    if !is_metadata_stable(candidate, observation)? {
        return Ok(StabilityOutcome {
            is_stable: false,
            digest: None,
        });
    }
    // TODO: add file footer verification for various file types
    differs_from_previous(state, candidate, observation).await
}

fn is_metadata_stable(candidate: &Candidate, observation: &Observation) -> Result<bool, ScanErr> {
    let latest = candidate.latest_observation()?;
    if latest.size != observation.size {
        return Ok(false);
    }
    if latest.mtime != observation.mtime {
        return Ok(false);
    }
    Ok(true)
}

async fn differs_from_previous(
    state: &State,
    candidate: &Candidate,
    observation: &Observation,
) -> Result<StabilityOutcome, ScanErr> {
    // check if there is a previous stable file
    let previous = if let Some(previous) = find_previous_stable_file(state, candidate) {
        previous
    } else {
        return Ok(StabilityOutcome {
            is_stable: true,
            digest: None,
        });
    };

    // check if stable file size has changed
    if previous.size != observation.size {
        return Ok(StabilityOutcome {
            is_stable: true,
            digest: None,
        });
    }

    // check if stable file digest has changed
    let digest = files::hash(&candidate.file).await?;
    Ok(StabilityOutcome {
        is_stable: true,
        digest: Some(digest),
    })
}

fn find_previous_stable_file(state: &State, candidate: &Candidate) -> Option<StableFile> {
    state
        .ledger
        .get(&candidate.file)
        .and_then(|stable_files| stable_files.last().cloned())
}
