// standard crates
use std::collections::{HashMap, HashSet};

// internal crates
use crate::cache::{self, dir_impl::entry_file_name, CacheEntry};
use crate::calibrations::{cleanup, LatestEntry};
use crate::disk::layout::CalibrationsLayout;
use crate::filesys::{files, File, FileSysErr, Overwrite};

// external crates
use tracing::error;
use uuid::Uuid;

// The latest calibrations are split like the config instances: the entries sit in
// one file, keyed by component, role and aspect, while each entry's data, which can
// be large, is its own file, keyed by report id.
pub type Calibrations = cache::FileCache<String, LatestEntry>;
pub type CalibrationContent = cache::DirCache<String, String>;

type Index = HashMap<String, CacheEntry<String, LatestEntry>>;

/// Move an `index` that can't be parsed aside, so the cache starts empty
/// rather than failing every report. The latest calibrations are a cache of
/// Miru, which the initial load refills.
pub async fn set_aside_corrupt_index(index: &File) {
    let Err(FileSysErr::ParseJSONErr(err)) = files::read_json::<Index>(index).await else {
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

/// Delete the content no index entry names, which a crash or a failed delete
/// leaves, so it never pushes the content cache past its cap and evicts live
/// content. Only file names are read, however large the content.
pub async fn prune_orphaned_contents(layout: &CalibrationsLayout) {
    let index = match files::read_json::<Index>(&layout.index()).await {
        Ok(index) => index,
        Err(FileSysErr::PathDoesNotExistErr(_)) => Index::new(),
        Err(err) => {
            error!("latest calibrations index could not be read to prune contents: {err}");
            return;
        }
    };
    let named: HashSet<String> = index
        .values()
        .map(|entry| entry_file_name(&entry.value.metadata.report_id.to_string()))
        .collect();
    cleanup::prune(&layout.contents(), &named).await;
}

async fn move_aside(index: &File) -> Result<File, FileSysErr> {
    let name = format!("{}.corrupt-{}", index.name()?, Uuid::new_v4());
    let aside = index.parent()?.file(&name);
    files::move_to(index, &aside, Overwrite::Deny).await?;
    Ok(aside)
}
