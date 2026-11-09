// standard crates
use std::collections::{HashMap, HashSet};

// internal crates
use crate::cache::{self, dir_impl::entry_file_name, CacheEntry};
use crate::calibrations::ReportMetadata;
use crate::disk::layout::LatestCalsLayout;
use crate::filesys::{dirs, files, FileSysErr};

// external crates
use tracing::error;

// The latest calibrations are split like the config instances: the entries sit in
// one file, keyed by component, role and aspect, while each entry's data, which can
// be large, is its own file, keyed by report id.
pub type LatestCals = cache::FileCache<String, ReportMetadata>;
pub type LatestCalContent = cache::DirCache<String, String>;

type Index = HashMap<String, CacheEntry<String, ReportMetadata>>;

/// Delete the content no index entry names, which a crash or a failed write or
/// delete leaves, so it does not count toward the content cache's cap after a
/// boot.
/// Only file names are read, however large the content.
pub async fn prune_orphaned_contents(layout: &LatestCalsLayout) {
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
        .map(|entry| entry_file_name(&entry.value.report_id.to_string()))
        .collect();
    dirs::delete_files_except(&layout.contents(), &named).await;
}
