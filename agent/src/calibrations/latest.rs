// standard crates
use std::collections::HashSet;

// internal crates
use crate::calibrations::{
    errors::{CalibrationNotFoundErr, CalibrationStoreFullErr, CalibrationsErr},
    report::ReportMetadata,
};
use crate::disk::layout::LatestCalsLayout;
use crate::filesys::{dirs, files, Dir, File, FileSysErr, WriteOptions};
use crate::trace;

// external crates
use tracing::{error, warn};
use uuid::Uuid;

/// The most keys the store holds in the agent.
pub const DEFAULT_CAPACITY: usize = 1000;

/// The entries, one per key, as `index.json` holds them.
type Index = Vec<ReportMetadata>;

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
        };
        store.prune_orphaned_contents().await;
        store
    }

    /// Make `metadata` its key's entry, with `data` as its content. A new key
    /// past the capacity is rejected rather than evicting another. A failure
    /// part way leaves at most content that no entry names, which the next
    /// open deletes. Puts must not overlap, since each rewrites the whole
    /// index.
    pub async fn put(&self, metadata: ReportMetadata, data: &str) -> Result<(), CalibrationsErr> {
        let mut index = self.read_index().await?;
        let replaced = take_entry(&mut index, &metadata);
        if replaced.is_none() {
            check_capacity(index.len(), self.capacity)?;
        }
        let report_id = metadata.report_id;
        self.write_content(report_id, data).await?;
        index.push(metadata);
        self.write_index(&index).await?;
        // the same report put again shares its content file, so keep it
        if let Some(replaced) = replaced.filter(|r| r.report_id != report_id) {
            self.delete_content(replaced.report_id).await;
        }
        Ok(())
    }

    /// The entries `filter` matches, ordered by component, role and aspect.
    pub async fn list(&self, filter: &Filter) -> Result<Vec<ReportMetadata>, CalibrationsErr> {
        let mut entries = self.read_index().await?;
        entries.retain(|metadata| filter.is_match(metadata));
        entries.sort_by(|a, b| key_parts(a).cmp(&key_parts(b)));
        Ok(entries)
    }

    /// The data of the entry for `report_id`. Not found unless an entry has
    /// that `report_id` and its content exists.
    pub async fn content(&self, report_id: Uuid) -> Result<String, CalibrationsErr> {
        let index = self.read_index().await?;
        if !index.iter().any(|m| m.report_id == report_id) {
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

    async fn write_index(&self, index: &Index) -> Result<(), FileSysErr> {
        files::write_json(&self.index, index, WriteOptions::OVERWRITE_ATOMIC).await
    }

    fn content_file(&self, report_id: Uuid) -> File {
        self.contents.file(&report_id.to_string())
    }

    async fn write_content(&self, report_id: Uuid, data: &str) -> Result<(), FileSysErr> {
        let file = self.content_file(report_id);
        files::write_string(&file, data, WriteOptions::OVERWRITE_ATOMIC).await
    }

    async fn delete_content(&self, report_id: Uuid) {
        files::delete_or_warn(&self.content_file(report_id)).await;
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
            .iter()
            .map(|metadata| metadata.report_id.to_string())
            .collect();
        dirs::delete_files_except(&self.contents, &named).await;
    }
}

/// Remove and return the entry with `metadata`'s key, if there is one.
fn take_entry(index: &mut Index, metadata: &ReportMetadata) -> Option<ReportMetadata> {
    let slot = index
        .iter()
        .position(|entry| key_parts(entry) == key_parts(metadata))?;
    Some(index.swap_remove(slot))
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

#[cfg(test)]
mod tests {
    // internal crates
    use super::{check_capacity, is_allowed, key_parts, not_found, take_entry, Index};
    use crate::test_utils::calibrations::report;
    use miru_agent::calibrations::{CalibrationsErr, ReportMetadata};

    // external crates
    use uuid::Uuid;

    /// A report for `component` and `role` with its own `report_id`.
    fn reported(component: &str, role: Option<&str>) -> ReportMetadata {
        ReportMetadata {
            report_id: Uuid::new_v4(),
            role: role.map(str::to_string),
            ..report(component).metadata
        }
    }

    // ============================ TAKE ENTRY ============================ //
    #[test]
    fn take_entry_removes_the_entry_with_the_same_key() {
        let arm = reported("arm", None);
        let left_arm = reported("arm", Some("left"));
        let mut index: Index = vec![arm.clone(), left_arm.clone()];
        let newer = ReportMetadata {
            report_id: Uuid::new_v4(),
            ..arm.clone()
        };

        let taken = take_entry(&mut index, &newer);

        assert_eq!(taken, Some(arm));
        assert_eq!(index, [left_arm]);
    }

    #[test]
    fn take_entry_leaves_the_index_for_a_new_key() {
        let arm = reported("arm", None);
        let mut index: Index = vec![arm.clone()];

        let taken = take_entry(&mut index, &reported("leg", None));

        assert_eq!(taken, None);
        assert_eq!(index, [arm]);
    }

    // ============================= KEY PARTS ============================ //
    #[test]
    fn key_parts_are_the_component_role_and_aspect_only() {
        let arm = reported("arm", None);
        // the same key from another report
        let newer = reported("arm", None);
        let left_arm = reported("arm", Some("left"));

        assert_eq!(key_parts(&arm), key_parts(&newer));
        assert_ne!(key_parts(&arm), key_parts(&left_arm));
    }

    // ============================ IS ALLOWED ============================ //
    #[test]
    fn is_allowed_allows_any_value_for_an_empty_list() {
        let cases: [(&[&str], &str, bool); 3] = [
            (&[], "arm", true),
            (&["arm", "leg"], "arm", true),
            (&["leg"], "arm", false),
        ];

        for (allowed, value, want) in cases {
            assert_eq!(is_allowed(allowed, &value), want, "{allowed:?} {value}");
        }
    }

    // ========================== CHECK CAPACITY ========================== //
    #[test]
    fn check_capacity_rejects_a_new_key_once_the_store_is_full() {
        // (size, capacity, accepted)
        let cases = [
            (0, 1, true),
            (1, 2, true),
            (1, 1, false),
            (2, 1, false),
            (0, 0, false),
        ];

        for (size, capacity, accepted) in cases {
            let result = check_capacity(size, capacity);
            match result {
                Ok(()) => assert!(accepted, "{size}/{capacity}"),
                Err(CalibrationsErr::StoreFullErr(err)) => {
                    assert!(!accepted, "{size}/{capacity}");
                    assert_eq!(err.capacity, capacity);
                }
                Err(err) => panic!("unexpected error: {err:?}"),
            }
        }
    }

    // ============================ NOT FOUND ============================= //
    #[test]
    fn not_found_names_the_report() {
        let report_id = Uuid::new_v4();

        let CalibrationsErr::NotFoundErr(err) = not_found(report_id) else {
            panic!("expected a not found error");
        };

        assert_eq!(err.report_id, report_id.to_string());
    }
}
