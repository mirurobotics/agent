// standard crates
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// internal crates
use crate::models;

// external crates
use chrono::{DateTime, TimeDelta, Utc};
use tracing::warn;

// ================================ CANDIDATES ===================================== //
/// A file matched by an active upload rule's glob during a scan, carrying the
/// smallest stability window among the rules that matched it.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub path: PathBuf,
    pub size: u64,
    pub modified_at: DateTime<Utc>,
    pub stability_window_secs: i32,
}

/// Enumerates the files matching each rule's `source.glob` and stats them.
/// Candidates are deduped by path; a file matched by multiple rules keeps the
/// smallest stability window. Invalid patterns and unreadable paths are skipped
/// with a warning so one bad rule cannot starve the others.
///
/// This walks the filesystem synchronously — call it from a blocking context
/// (e.g. `tokio::task::spawn_blocking`), never directly on the async runtime.
pub fn discover_blocking(rules: &[models::UploadRule]) -> Vec<Candidate> {
    let mut by_path: HashMap<PathBuf, Candidate> = HashMap::new();
    for rule in rules {
        let paths = match glob::glob(&rule.source.glob) {
            Ok(paths) => paths,
            Err(e) => {
                warn!(
                    "upload rule '{}' has an invalid glob '{}': {e}",
                    rule.id, rule.source.glob
                );
                continue;
            }
        };
        for entry in paths {
            let path = match entry {
                Ok(path) => path,
                Err(e) => {
                    warn!(
                        "skipping unreadable path while matching glob '{}': {e}",
                        rule.source.glob
                    );
                    continue;
                }
            };
            let Some(candidate) = stat(path, rule.source.stability_window_secs) else {
                continue;
            };
            by_path
                .entry(candidate.path.clone())
                .and_modify(|existing| {
                    existing.stability_window_secs = existing
                        .stability_window_secs
                        .min(candidate.stability_window_secs)
                })
                .or_insert(candidate);
        }
    }
    by_path.into_values().collect()
}

fn stat(path: PathBuf, stability_window_secs: i32) -> Option<Candidate> {
    let metadata = match std::fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(e) => {
            warn!("failed to stat '{}': {e}", path.display());
            return None;
        }
    };
    if !metadata.is_file() {
        return None;
    }
    let modified_at = match metadata.modified() {
        Ok(modified_at) => DateTime::<Utc>::from(modified_at),
        Err(e) => {
            warn!("failed to read the mtime of '{}': {e}", path.display());
            return None;
        }
    };
    Some(Candidate {
        path,
        size: metadata.len(),
        modified_at,
        stability_window_secs,
    })
}

// ================================ READINESS ====================================== //
/// The last observed size and mtime of a matched file, plus when that exact
/// (size, mtime) pair was first observed.
#[derive(Clone, Debug, PartialEq)]
pub struct Observation {
    pub size: u64,
    pub modified_at: DateTime<Utc>,
    pub unchanged_since: DateTime<Utc>,
}

/// Folds a fresh stat into the per-file observation state: an unchanged
/// (size, mtime) pair keeps its `unchanged_since` watermark; any change (or a
/// first sighting) restarts it at `now`.
pub fn observe(
    prev: Option<&Observation>,
    size: u64,
    modified_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Observation {
    match prev {
        Some(prev) if prev.size == size && prev.modified_at == modified_at => prev.clone(),
        _ => Observation {
            size,
            modified_at,
            unchanged_since: now,
        },
    }
}

/// A file is ready once its size and mtime have both been observed unchanged
/// for at least the stability window. Negative windows are treated as zero
/// (immediately ready).
pub fn is_ready(observation: &Observation, stability_window_secs: i32, now: DateTime<Utc>) -> bool {
    let window = TimeDelta::seconds(i64::from(stability_window_secs.max(0)));
    now.signed_duration_since(observation.unchanged_since) >= window
}

/// Finalization-marker hook for format-aware readiness checks (e.g. an MCAP
/// end-of-file magic or a parquet footer proving the writer finished the
/// file). M2 has no marker detection, so every quiesced file passes; a later
/// milestone plugs real detection in here without touching the scan flow.
pub fn passes_finalization_markers(_path: &Path) -> bool {
    true
}
