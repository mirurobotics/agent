// standard crates
use std::collections::HashMap;

// internal crates
use crate::cache::{self, CacheEntry};
use crate::calibrations::LatestEntry;
use crate::filesys::{files, File, FileSysErr, Overwrite};

// external crates
use tracing::error;
use uuid::Uuid;

// The latest calibrations are split like the config instances: the entries sit in
// one file, keyed by component, role and aspect, while each entry's data, which can
// be large, is its own file, keyed by report id.
pub type Calibrations = cache::FileCache<String, LatestEntry>;
pub type CalibrationContent = cache::DirCache<String, String>;

/// Move an `index` that can't be parsed aside, so the cache starts empty
/// rather than failing every report. The latest calibrations are a cache of
/// Miru, which the initial load refills.
pub async fn set_aside_corrupt_index(index: &File) {
    let read = files::read_json::<HashMap<String, CacheEntry<String, LatestEntry>>>(index);
    let Err(FileSysErr::ParseJSONErr(err)) = read.await else {
        return;
    };
    match move_aside(index).await {
        Ok(aside) => error!(
            "latest calibrations index {index} could not be parsed ({err}); moved it to {aside} \
             and started empty"
        ),
        Err(move_err) => error!(
            "latest calibrations index {index} could not be parsed ({err}) or moved aside \
             ({move_err})"
        ),
    }
}

async fn move_aside(index: &File) -> Result<File, FileSysErr> {
    let name = format!("{}.corrupt-{}", index.name()?, Uuid::new_v4());
    let aside = index.parent()?.file(&name);
    files::move_to(index, &aside, Overwrite::Deny).await?;
    Ok(aside)
}
