// standard crates
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

// internal crates
use crate::calibrations::{
    errors::{CalibrationNotFoundErr, CalibrationStoreFullErr, CalibrationsErr},
    report::ReportMetadata,
};
use crate::disk::layout::LatestCalsLayout;
use crate::filesys::{dirs, files, Dir, File, FileSysErr, WriteOptions};
use crate::trace;

// external crates
use tokio::sync::Mutex;
use tracing::{error, warn};
use uuid::Uuid;

/// The most keys the store holds in the agent.
pub const DEFAULT_CAPACITY: usize = 1000;

/// The entries by key, as `index.json` holds them.
type Index = HashMap<String, ReportMetadata>;

/// Which entries to list. Empty means any; `None` in `role`/`aspect` matches an
/// entry without one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filter {
    pub component: Vec<String>,
    pub role: Vec<Option<String>>,
    pub aspect: Vec<Option<String>>,
}

impl Filter {
    pub fn is_match(&self, metadata: &ReportMetadata) -> bool {
        is_allowed(&self.component, &metadata.component)
            && is_allowed(&self.role, &metadata.role)
            && is_allowed(&self.aspect, &metadata.aspect)
    }
}

/// The newest calibration for each component, role and aspect: one entry per
/// key in `index.json`, and each entry's data in its own file under `contents/`,
/// named by its `report_id`. A report replaces its key's entry, and a new key
/// past the capacity is rejected rather than evicting another.
#[derive(Clone, Debug)]
pub struct LatestStore {
    index: File,
    contents: Dir,
    capacity: usize,
    // serializes puts, which read, change and rewrite the index
    write_lock: Arc<Mutex<()>>,
}

impl LatestStore {
    /// Open the store in `layout`, holding at most `capacity` keys. It first
    /// deletes the content no entry names, which a crash or a failed write or
    /// delete leaves.
    pub async fn open(layout: LatestCalsLayout, capacity: usize) -> Self {
        let store = Self {
            index: layout.index(),
            contents: layout.contents(),
            capacity,
            write_lock: Arc::new(Mutex::new(())),
        };
        store.prune_orphaned_contents().await;
        store
    }

    /// Make `metadata` its key's entry, with `data` as its content. A new key
    /// past the capacity is rejected rather than evicting another. A failure
    /// part way leaves at most content that no entry names, which the next
    /// open deletes.
    pub async fn put(&self, metadata: ReportMetadata, data: &str) -> Result<(), CalibrationsErr> {
        let _guard = self.write_lock.lock().await;
        let mut index = self.read_index().await?;
        let key = key(&metadata);
        if !index.contains_key(&key) {
            check_capacity(index.len(), self.capacity)?;
        }
        let report_id = metadata.report_id;
        let content = self.content_file(report_id);
        files::write_string(&content, data, WriteOptions::OVERWRITE_ATOMIC).await?;
        let replaced = index.insert(key, metadata);
        files::write_json(&self.index, &index, WriteOptions::OVERWRITE_ATOMIC).await?;
        // the same report put again shares its content file, so keep it
        if let Some(replaced) = replaced.filter(|r| r.report_id != report_id) {
            files::delete_or_warn(&self.content_file(replaced.report_id)).await;
        }
        Ok(())
    }

    /// The entries `filter` matches, ordered by component, role and aspect.
    pub async fn list(&self, filter: &Filter) -> Result<Vec<ReportMetadata>, CalibrationsErr> {
        let index = self.read_index().await?;
        let mut entries: Vec<ReportMetadata> = index.into_values().collect();
        entries.retain(|metadata| filter.is_match(metadata));
        entries.sort_by(|a, b| key_parts(a).cmp(&key_parts(b)));
        Ok(entries)
    }

    /// The data of the entry for `report_id`. Not found unless an entry has
    /// that `report_id` and its content exists.
    pub async fn content(&self, report_id: Uuid) -> Result<String, CalibrationsErr> {
        let index = self.read_index().await?;
        if !index.values().any(|m| m.report_id == report_id) {
            return Err(not_found(report_id));
        }
        match files::read_string(&self.content_file(report_id)).await {
            Err(FileSysErr::PathDoesNotExistErr(_)) => Err(not_found(report_id)),
            result => Ok(result?),
        }
    }

    /// The index, which is empty until the first put writes it.
    async fn read_index(&self) -> Result<Index, CalibrationsErr> {
        match files::read_json::<Index>(&self.index).await {
            Err(FileSysErr::PathDoesNotExistErr(_)) => Ok(Index::new()),
            result => Ok(result?),
        }
    }

    fn content_file(&self, report_id: Uuid) -> File {
        self.contents.file(&report_id.to_string())
    }

    /// Delete the content no entry names. If the index can't be read, nothing
    /// is deleted.
    async fn prune_orphaned_contents(&self) {
        let index = match self.read_index().await {
            Ok(index) => index,
            Err(e) => {
                error!("calibrations: not pruning latest contents, index unreadable: {e}");
                return;
            }
        };
        let named: HashSet<String> = index
            .values()
            .map(|metadata| metadata.report_id.to_string())
            .collect();
        dirs::delete_files_except(&self.contents, &named).await;
    }
}

/// The index's key for `metadata`: its component, role and aspect as
/// a JSON array, so no two keys collide.
fn key(metadata: &ReportMetadata) -> String {
    serde_json::json!(key_parts(metadata)).to_string()
}

fn key_parts(metadata: &ReportMetadata) -> (&String, &Option<String>, &Option<String>) {
    (&metadata.component, &metadata.role, &metadata.aspect)
}

/// Whether `value` is in `allowed`, where an empty list allows any value.
fn is_allowed<T: PartialEq>(allowed: &[T], value: &T) -> bool {
    allowed.is_empty() || allowed.contains(value)
}

/// Reject a new key once the store holds `capacity` keys.
fn check_capacity(size: usize, capacity: usize) -> Result<(), CalibrationsErr> {
    if size < capacity {
        return Ok(());
    }
    let err = CalibrationStoreFullErr {
        capacity,
        trace: trace!(),
    };
    warn!("calibrations: rejecting report: {err}");
    Err(err.into())
}

fn not_found(report_id: Uuid) -> CalibrationsErr {
    CalibrationsErr::NotFoundErr(CalibrationNotFoundErr {
        report_id: report_id.to_string(),
        trace: trace!(),
    })
}
