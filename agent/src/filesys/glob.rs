// standard crates
use std::time::SystemTime;

// internal crates
use crate::filesys::File;

// external crates
use tracing::error;

/// Expand a glob pattern and stat each matching regular file.
///
/// Returns `(File, size, mtime)` for every match that is a readable regular
/// file. An invalid glob pattern is logged via `error!` and yields an empty
/// vec. Non-files and entries whose metadata or modified time cannot be read
/// mid-scan are skipped.
pub fn stat_matches(pattern: &str) -> Vec<(File, u64, SystemTime)> {
    let mut matches = Vec::new();
    let paths = match glob::glob(pattern) {
        Ok(paths) => paths,
        Err(e) => {
            error!("invalid glob {:?}: {e}", pattern);
            return matches;
        }
    };
    for entry in paths {
        let path = match entry {
            Ok(p) => p,
            Err(_) => continue,
        };
        if !path.is_file() {
            continue;
        }
        let meta = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let size = meta.len();
        let mtime = match meta.modified() {
            Ok(t) => t,
            Err(_) => continue,
        };
        matches.push((File::new(path), size, mtime));
    }
    matches
}
