// standard crates
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

// internal crates
use crate::filesys::{files, File, PathExt};
use crate::models::UploadRule;
use crate::trace;
use crate::upload::errors::*;

// external crates
use chrono::{DateTime, TimeDelta, Utc};
use tracing::error;

/// Per-file stability state used by the scanner. A file is "stable" once
/// its (size, mtime) have been unchanged for at least `stability_window_secs` since
/// `stable_since`. Fields are public so unit tests can construct/inspect observations
/// against the pure `find_stable` seam directly.
pub struct Observation {
    pub size: u64,
    pub mtime: std::time::SystemTime,
    pub stable_since: DateTime<Utc>,
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
    observations: HashMap<File, Observation>,
    ledger: HashSet<File>,
}

impl State {
    pub(crate) fn new(rule: UploadRule) -> Self {
        Self {
            rule,
            observations: HashMap::new(),
            ledger: HashSet::new(),
        }
    }

    fn rule(&self) -> &UploadRule {
        &self.rule
    }

    fn ledger_count(&self) -> usize {
        self.ledger.len()
    }

    fn set_rule(&mut self, rule: UploadRule) -> Result<(), UploadErr> {
        if self.rule.upload_collection_id != rule.upload_collection_id {
            return Err(UploadErr::InvalidRule(InvalidRule {
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
    next_scan_at: Option<DateTime<Utc>>,
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
        Self {
            next_scan_at: None,
            state,
        }
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
    pub(crate) fn set_rule(&mut self, rule: UploadRule) -> Result<(), UploadErr> {
        self.state.set_rule(rule)
    }

    /// Per-collection cadence due check: due when it has no recorded next-scan
    /// time (freshly added) or its next-scan time has arrived.
    pub(crate) fn is_due(&self, now: DateTime<Utc>) -> bool {
        self.next_scan_at.is_none_or(|next| next <= now)
    }

    /// Schedule this collection's next scan one global interval out.
    pub(crate) fn reschedule(&mut self, now: DateTime<Utc>, min_poll_interval_secs: i64) {
        self.next_scan_at = Some(now + TimeDelta::seconds(min_poll_interval_secs));
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
        let moved_observations = std::mem::take(&mut self.state.observations);
        let moved_ledger = std::mem::take(&mut self.state.ledger);
        let rule = self.state.rule.clone();
        let (returned_observations, returned_ledger, stable) =
            match tokio::task::spawn_blocking(move || {
                let mut obs = moved_observations;
                let mut ledger = moved_ledger;
                let stable = find_stable(&rule, &mut obs, &mut ledger, now);
                (obs, ledger, stable)
            })
            .await
            {
                Ok(result) => result,
                Err(e) => {
                    error!("uploads stability task panicked: {e:?}");
                    (HashMap::new(), HashSet::new(), Vec::new())
                }
            };
        self.state.observations = returned_observations;
        self.state.ledger = returned_ledger;
        stable
    }
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
pub fn find_stable(
    rule: &UploadRule,
    observations: &mut HashMap<File, Observation>,
    ledger: &mut HashSet<File>,
    now: DateTime<Utc>,
) -> Vec<StableFile> {
    let mut stable = Vec::new();

    // Enumerate the glob. An invalid pattern is logged and treated as no matches
    // so the scanner never crashes on a bad rule.
    let matched = match files::glob(&rule.source.glob) {
        Ok(matched) => matched,
        Err(e) => {
            error!("invalid glob {:?}: {e}", rule.source.glob);
            return stable;
        }
    };

    for file in matched {
        // Stat each globbed file with SYNC std::fs (this runs under
        // spawn_blocking; the async files::size/last_modified must NOT be used
        // here). Skip non-files and entries whose metadata/mtime cannot be read
        // (e.g. a file deleted mid-scan).
        let meta = match std::fs::metadata(file.path()) {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        if !meta.is_file() {
            continue;
        }
        let size = meta.len();
        let mtime = match meta.modified() {
            Ok(mtime) => mtime,
            Err(_) => continue,
        };

        if let Some(rf) = decide_file(
            file,
            size,
            mtime,
            rule.source.stability_window_secs,
            observations,
            ledger,
            now,
        ) {
            stable.push(rf);
        }
    }

    stable
}

/// Advance the stability window for one file and decide whether it is newly stable.
/// When (size, mtime) are unchanged since the last observation and the window has
/// elapsed, the file is recorded in `ledger` and returned once.
/// Otherwise the window is (re)started by inserting a fresh observation and nothing is
/// returned.
fn decide_file(
    file: File,
    size: u64,
    mtime: std::time::SystemTime,
    stability_window_secs: i32,
    observations: &mut HashMap<File, Observation>,
    ledger: &mut HashSet<File>,
    now: DateTime<Utc>,
) -> Option<StableFile> {
    match observations.get(&file) {
        // size/mtime unchanged since last observation: the stability window
        // is still running; report once it has elapsed.
        Some(obs) if obs.size == size && obs.mtime == mtime => {
            // HOOK (M3): finalization-marker detection (MCAP footer / parquet
            // magic bytes) would gate stability here in addition to size+mtime
            // stability. NOT implemented in M2.
            if now.signed_duration_since(obs.stable_since).num_seconds()
                >= stability_window_secs as i64
                && !ledger.contains(&file)
            {
                ledger.insert(file.clone());
                return Some(StableFile {
                    path: file.path().clone(),
                    modified_at: mtime.into(),
                });
            }
            None
        }
        // new file, or size/mtime changed since last observation: (re)start
        // the stability window and do NOT report.
        _ => {
            observations.insert(
                file,
                Observation {
                    size,
                    mtime,
                    stable_since: now,
                },
            );
            None
        }
    }
}
