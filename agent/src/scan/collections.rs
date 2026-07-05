// standard crates
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

// internal crates
use crate::filesys::{files, File};
use crate::filesys::PathExt;
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

pub struct Candidate {
    pub file: File,
    pub latest_observation: Observation,
    pub observations: Vec<Observation>,
    pub stable_since: DateTime<Utc>,
}

pub struct Upload {
    pub file: File,
    pub size: u64,
    pub digest: String,
    pub modified_at: DateTime<Utc>,
}

/// A file that has been determined stable enough for upload. Fields are public
/// so unit tests can assert against the pure `find_stable` seam directly.
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
        self.rule = rule;
        Ok(())
    }
}

/// Owned (non-actor) sub-scanner for a single upload collection. Holds exactly
/// one active `UploadRule` plus that collection's own observation/dedupe/cadence
/// state. This is the structural home of what used to be global scanner state:
/// each collection now advances its stability state machine independently.
pub(crate) struct CollectionScanner {
    state: State,
}

impl CollectionScanner {
    /// New sub-scanner with empty state and no recorded next-scan time, so it is
    /// due on its first scan.
    pub(crate) fn new(rule: UploadRule) -> Self {
        Self::from_state(State::new(rule))
    }

    /// Sub-scanner initialized from existing per-collection state. Cadence starts
    /// unset, so the collection is due on its first scan.
    pub(crate) fn from_state(state: State) -> Self {
        Self { state }
    }

    pub(crate) fn rule(&self) -> &UploadRule {
        self.state.rule()
    }

    pub(crate) fn ledger_count(&self) -> usize {
        self.state.ledger_count()
    }

    /// Replace only the active rule, carrying over observation/dedupe/cadence
    /// state. This is the whole point of the collection-keyed partition: a rule
    /// redeploy (new rule id / digest for the same collection) preserves the
    /// in-flight stability windows and dedupe set for that collection.
    pub(crate) fn set_rule(&mut self, rule: UploadRule) -> Result<(), ScanErr> {
        self.state.set_rule(rule)
    }

    /// Scan this collection's globbed files for newly stable uploads under
    /// `spawn_blocking`.
    ///
    /// `find_stable` does blocking fs I/O (glob enumeration + stat). Per the Decision
    /// Log, run it under spawn_blocking to keep the runtime responsive. Because
    /// `spawn_blocking` needs `'static`, we move this collection's state into the
    /// blocking task and take it back out afterwards (alongside the newly-stable
    /// files) so find_stable itself stays a plain, unit-testable sync fn. If the
    /// task panics we log and restore empty observation/ledger maps while keeping
    /// the active rule.
    pub(crate) async fn scan(&mut self, now: DateTime<Utc>) -> Vec<StableFile> {
        let mut moved_state = std::mem::take(&mut self.state);
        let (returned_state, stable) =
            match tokio::task::spawn_blocking(move || {
                let mut state = moved_state;
                let stable = find_stable(&mut state, now);
                (state, stable)
            })
            .await
            {
                Ok(result) => result,
                Err(e) => {
                    error!("uploads stability task panicked: {e:?}");
                    (State::default(), Vec::new())
                }
            };
        self.state = returned_state;
        stable
    }
}

pub async fn observe_file(file: &File) -> Result<Observation, ScanErr>  {
    let meta = files::metadata(file).await?;
    let mtime = meta.modified()?;
    Ok(Observation { size: meta.len(), mtime })
}

/// Pure stability decision for a single rule. Enumerates `rule.source.glob`, stats each
/// matched file, updates the per-file stability state in `observations`, and returns
/// the files that have been size/mtime-stable for at least `stability_window_secs`.
///
/// This does BLOCKING filesystem I/O (glob walk + metadata) and is intended to be
/// called inside `tokio::task::spawn_blocking`. It is kept sync and pure (state in/out
/// via the `observations`/`ledger` arguments) so it can be unit-tested
/// directly without a runtime.
///
/// Returns only NEWLY-stable files: a file crossing into "stable" is recorded in
/// `ledger` and is never returned again, so callers can treat every returned
/// file as a once-per-file event.
pub async fn find_stable(state: &mut State, now: DateTime<Utc>) -> Result<Vec<StableFile>, ScanErr> {
    let mut stable = Vec::new();

    // Enumerate the glob. An invalid pattern is logged and treated as no matches
    // so the scanner never crashes on a bad rule.
    let matched = match files::glob(&state.rule.source.glob) {
        Ok(matched) => matched,
        Err(e) => {
            error!("invalid glob {:?}: {e}", state.rule.source.glob);
            return stable;
        }
    };

    for file in matched {
        let observation = observe_file(&file).await?;
        if let Some(rf) = decide_file_stability(state, file, observation, now).await? {
            stable.push(rf);
        }
    }

    stable
}

async fn decide_file_stability(
    state: &mut State,
    file: File,
    observation: Observation,
    now: DateTime<Utc>,
) -> Option<StableFile> {
    match state.candidates.get(&file) {
        // new file, or size/mtime changed since last observation: (re)start
        // the stability window and do NOT report.
        _ => {
            state.candidates.insert(file,
                Candidate {
                    file,
                    latest_observation: observation,
                    observations: vec![observation],
                    stable_since: now,
                });
            None
        }
        // size/mtime unchanged since last observation: the stability window
        // is still running; report once it has elapsed.
        Some(candidate) => {
            decide_candidate_stability(state, now, candidate).await
        }

    }
}

async fn is_stable(
    state: &mut State,
    candidate: &Candidate,
    observation: Observation,
    now: DateTime<Utc>,
) -> bool {

    // cheap checks: size & mtime
    let latest = candidate.latest_observation;
    let latest.size != observation.size {
        return false;
    }
    if latest.mtime != observation.mtime {
        return false;
    }
    true

}


async fn decide_candidate_stability(
    state: &mut State,
    candidate: &Candidate,
    now: DateTime<Utc>,
) -> Option<StableFile> {
    // HOOK (M3): finalization-marker detection (MCAP footer / parquet // magic bytes) would gate stability here in addition to size+mtime
    // stability. NOT implemented in M2.
    let stab_win_secs = state.rule.source.stability_window_secs as i64;
    if now.signed_duration_since(candidate.stable_since).num_seconds()
        >= stab_win_secs
        && !state.ledger.contains_key(&candidate.file)
    {
        return Some(StableFile {
            path: candidate.file.path().clone(),
            modified_at: candidate.observations.last().unwrap().mtime.into(),
        });
    }
    None
}
