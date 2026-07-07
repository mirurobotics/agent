// standard crates
use std::collections::HashMap;

// internal crates
use crate::filesys::errors::*;
use crate::filesys::{files, File, PathExt};
use crate::models::{Deployment, UploadRule};
use crate::scan::errors::*;
use crate::trace;

// external crates
use chrono::{DateTime, Utc};

pub struct State {
    cfg: Config,
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
                .is_none_or(|stable_file| stable_file.modified_at >= before)
        });
        Ok(())
    }
}

#[derive(Clone)]
pub struct Observation {
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
    pub stable_since: DateTime<Utc>,
}

impl Candidate {
    pub fn latest_observation(&self) -> Result<Observation, ScanErr> {
        self.observations.last().cloned().ok_or(ScanErr::InternalError(InternalError {
            message: "No observations found".to_string(),
            trace: trace!(),
        }))
    }

    pub fn first_observation(&self) -> Result<Observation, ScanErr> {
        self.observations.first().cloned().ok_or(ScanErr::InternalError(InternalError {
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
    pub modified_at: DateTime<Utc>,
    pub deployment_id: String,
    pub upload_rule_id: String,
}

/// Owned (non-actor) sub-scanner for a single upload collection. 
pub(crate) struct CollectionScanner {
    state: State,
}

impl CollectionScanner {
    pub(crate) fn new(config: Config) -> Self {
        Self::from_state(State::new(config))
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
    pub(crate) fn set_config(&mut self, config: Config) -> Result<(), ScanErr> {
        self.state.set_config(config)
    }

    pub async fn evaluate_candidates(
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
                },
                EvalAction::Stable(stable_file) => {
                    stable_files.push(stable_file.clone());
                    self.state.ledger.insert(candidate.file.clone(), vec![stable_file]);
                },
            }
        }
        Ok(stable_files)
    }

    pub async fn discover_candidates(
        &mut self,
        now: DateTime<Utc>,
    ) -> Result<Vec<StableFile>, ScanErr> {
        for file in files::glob(&self.state.cfg.rule.source.glob)? {
            if self.state.candidates.contains_key(&file) {
                continue;
            }
            self.state.candidates.insert(file.clone(), Candidate {
                file,
                observations: Vec::new(),
                stable_since: now,
            });
        }
        Ok(Vec::new())
    }

    fn prune_ledger(&mut self, before: DateTime<Utc>) -> Result<(), ScanErr> {
        self.state.prune_ledger(before)
    }
}

pub async fn observe_file(state: &State, file: &File) -> Result<Observation, ScanErr>  {
    let meta = files::metadata(file).await?;
    let mtime = meta.modified().map_err(|source| {
        ScanErr::FileSysErr(FileSysErr::FileMetadataErr(FileMetadataErr {
            file: file.clone(),
            source: Box::new(source),
            trace: trace!(),
        }))
    })?;
    Ok(Observation { 
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

    if !has_stability_window_elapsed(state, candidate, now) {
        return Ok(EvalAction::WaitForStabilityWindow);
    }

    if !candidate.file.exists() {
        // file was deleted
        return Ok(EvalAction::Unstable);
    }

    let observation = observe_file(state, &candidate.file).await?;
    let outcome = determine_stability(state, candidate, &observation).await?;

    if outcome.is_stable {
        let first = candidate.first_observation()?;
        let last = candidate.latest_observation()?;
        debug_assert!(first.equal_metadata(&last));
        Ok(EvalAction::Stable(StableFile {
                file: candidate.file.clone(),
                size: observation.size,
                digest: outcome.digest()?,
                modified_at: last.mtime.into(),
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
) -> bool {
    let window = state.cfg.rule.source.stability_window_secs as i64;
    now.signed_duration_since(candidate.stable_since).num_seconds() >= window
}

pub struct StabilityOutcome {
    pub is_stable: bool,
    pub digest: Option<String>,
}

impl StabilityOutcome {
    pub fn digest(&self) -> Result<String, ScanErr> {
        self.digest.clone().ok_or(ScanErr::InternalError(InternalError {
            message: "No digest found".to_string(),
            trace: trace!(),
        }))
    }
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

fn is_metadata_stable(
    candidate: &Candidate,
    observation: &Observation,
) -> Result<bool, ScanErr> {
    if candidate.latest_observation()?.size != observation.size {
        return Ok(false);
    }
    if candidate.latest_observation()?.mtime != observation.mtime {
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
    state.ledger.get(&candidate.file).and_then(|stable_files| stable_files.last().cloned())
}
