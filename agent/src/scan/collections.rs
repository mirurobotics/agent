// standard crates
use std::collections::HashMap;
use std::path::PathBuf;

// internal crates
use crate::filesys::errors::*;
use crate::filesys::{files, File, PathExt};
use crate::models::UploadRule;
use crate::scan::errors::*;
use crate::trace;

// external crates
use chrono::{DateTime, Utc};
use tracing::error;

pub struct Observation {
    pub size: u64,
    pub mtime: std::time::SystemTime,
}

// A file that is a candidate for upload.
pub struct Candidate {
    pub file: File,
    pub latest_observation: Observation,
    pub observations: Vec<Observation>,
    pub stable_since: DateTime<Utc>,
}

// A file that is marked for upload
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upload {
    pub file: File,
    pub size: u64,
    pub digest: String,
    pub modified_at: DateTime<Utc>,
}

/// A file that has been determined stable enough for upload. 
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StableFile {
    pub path: PathBuf,
    pub modified_at: DateTime<Utc>,
}

pub struct State {
    rule: UploadRule,
    candidates: HashMap<File, Candidate>,
    ledger: HashMap<File, Vec<Upload>>,
}

impl Default for State {
    fn default() -> Self {
        Self::new(UploadRule::default())
    }
}

impl State {
    pub(crate) fn new(rule: UploadRule) -> Self {
        Self {
            rule,
            candidates: HashMap::new(),
            ledger: HashMap::new(),
        }
    }

    fn rule(&self) -> &UploadRule {
        &self.rule
    }

    fn ledger_count(&self) -> usize {
        self.ledger.len()
    }

    fn set_rule(&mut self, rule: UploadRule) -> Result<(), ScanErr> {
        if self.rule.upload_collection_id != rule.upload_collection_id {
            return Err(ScanErr::InvalidRule(InvalidRule {
                existing_upload_collection_id: self.rule.upload_collection_id.clone(),
                replacement_upload_collection_id: rule.upload_collection_id,
                trace: trace!(),
            }));
        }

        // FIXME: when a new rule is set, we need to prune the candidates and ledger
        // accordingly

        self.rule = rule;
        Ok(())
    }
}

/// Owned (non-actor) sub-scanner for a single upload collection. 
pub(crate) struct CollectionScanner {
    state: State,
}

impl CollectionScanner {
    pub(crate) fn new(rule: UploadRule) -> Self {
        Self::from_state(State::new(rule))
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

    /// Replace only the active rule, carrying over observation/dedupe/cadence state. 
    pub(crate) fn set_rule(&mut self, rule: UploadRule) -> Result<(), ScanErr> {
        self.state.set_rule(rule)
    }

    /// Scan this collection's globbed files for newly stable uploads
    pub(crate) async fn scan(&mut self, now: DateTime<Utc>) -> Vec<StableFile> {
        match scan(&mut self.state, now).await {
            Ok(stable) => stable,
            Err(e) => {
                error!("find_stable failed: {e}");
                Vec::new()
            }
        }
    }
}

pub async fn scan(
    state: &mut State,
    now: DateTime<Utc>,
) -> Result<Vec<StableFile>, ScanErr> {
    let mut stable = Vec::new();

    let matched = files::glob(&state.rule.source.glob)?;
    for file in matched {
        let observation = observe_file(&file).await?;
        if let Some(rf) = decide_file_stability(state, file, observation, now).await {
            stable.push(rf);
        }
    }

    Ok(stable)
}

pub async fn observe_file(file: &File) -> Result<Observation, ScanErr>  {
    let meta = files::metadata(file).await?;
    let mtime = meta.modified().map_err(|source| {
        ScanErr::FileSysErr(FileSysErr::FileMetadataErr(FileMetadataErr {
            file: file.clone(),
            source: Box::new(source),
            trace: trace!(),
        }))
    })?;
    Ok(Observation { size: meta.len(), mtime })
}

async fn decide_file_stability(
    state: &mut State,
    file: File,
    observation: Observation,
    now: DateTime<Utc>,
) -> Option<StableFile> {
    let candidate = match state.candidates.get(&file) {
        // new candidate -> not stable
        _ => {
            let candidate = Candidate {
                file,
                latest_observation: observation,
                observations: vec![observation],
                stable_since: now,
            };
            state.candidates.insert(file, candidate);
            return None;
        }
        Some(candidate) => candidate,
    };

    if is_stable(state, &candidate, observation, now).await {
        Some(StableFile {
            path: candidate.file.path().clone(),
            modified_at: candidate.observations.last().unwrap().mtime.into(),
        })
    } else {
        None
    }
}

async fn is_stable(
    state: &mut State,
    candidate: &Candidate,
    observation: Observation,
    now: DateTime<Utc>,
) -> bool {
    if !has_stability_window_elapsed(state, candidate, now) {
        return false;
    }
    if !is_metadata_stable(candidate, observation) {
        return false;
    }
    differs_from_previous(state, candidate, observation).await
}

fn has_stability_window_elapsed(
    state: &State,
    candidate: &Candidate,
    now: DateTime<Utc>,
) -> bool {
    let window = state.rule.source.stability_window_secs as i64;
    now.signed_duration_since(candidate.stable_since).num_seconds() >= window
}

fn is_metadata_stable(
    candidate: &Candidate,
    observation: Observation,
) -> bool {
    if candidate.latest_observation.size != observation.size {
        return false;
    }
    if candidate.latest_observation.mtime != observation.mtime {
        return false;
    }
    true
}

fn differs_from_previous(
    state: &State,
    candidate: &Candidate,
    observation: Observation,
) -> bool {
    // check if there is a previous upload
    let previous = if let Some(previous) = find_previous_upload(state, candidate) {
        previous
    } else {
        return true;
    };

    // check if upload size has changed
    if previous.size != observation.size {
        return true;
    }

    // check if upload digest has changed
    if candidate.digest != observation.mtime {
        return false;
    }
    true
}

fn find_previous_upload(state: &State, candidate: &Candidate) -> Option<Upload> {
    state.ledger.get(&candidate.file).and_then(|uploads| uploads.last().cloned())
}
