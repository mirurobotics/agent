// standard crates
use std::collections::HashSet;

// internal crates
use crate::filesys::{dirs, files, Dir, File, PathExt};

// external crates
use tracing::warn;

/// Delete the files in `dir` whose names `kept` doesn't hold. Failures are
/// logged and left for the next boot, and a missing `dir` is not created.
pub(crate) async fn prune(dir: &Dir, kept: &HashSet<String>) {
    if !dir.exists() {
        return;
    }
    let listing = match dirs::files(dir).await {
        Ok(listing) => listing,
        Err(e) => {
            warn!("calibrations: failed to list {dir}: {e}");
            return;
        }
    };
    for file in listing {
        if file.name().is_ok_and(|name| kept.contains(name)) {
            continue;
        }
        warn!("calibrations: deleting {file}, which nothing names");
        delete(&file).await;
    }
}

/// Delete `file`, logging a failure. A file that is already gone is fine.
pub(crate) async fn delete(file: &File) {
    if let Err(e) = files::delete(file).await {
        warn!("calibrations: failed to delete {file}: {e}");
    }
}
