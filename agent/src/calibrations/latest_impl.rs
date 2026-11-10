// standard crates
use std::collections::HashSet;

// internal crates
use crate::calibrations::{
    errors::{CalibrationNotFoundErr, CalibrationStoreFullErr, CalibrationsErr},
    report::{Report, ReportMetadata},
};
use crate::disk::layout::LatestCalsLayout;
use crate::filesys::{dirs, files, Dir, File, FileSysErr, WriteOptions};
use crate::trace;

// external crates
use serde::{Deserialize, Serialize};
use tracing::{error, warn};
use uuid::Uuid;

/// The entries, one per key.
type Index = Vec<ReportMetadata>;

/// `index.json`. An object, as `queue.json` is, so a field for the whole store
/// can be added without a reader for two shapes.
#[derive(Default, Deserialize, Serialize)]
struct IndexFile {
    entries: Index,
}

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
        matches(&self.component, &metadata.component)
            && matches(&self.role, &metadata.role)
            && matches(&self.aspect, &metadata.aspect)
    }
}

/// The newest calibration for each component, role and aspect: one entry per
/// key in `index.json`, and each entry's data in its own file under `contents/`,
/// named by its `report_id`. A report replaces its key's entry, and a new key
/// past the capacity is rejected rather than evicting another.
/// [`LatestStore`]'s worker owns it, so it is the store's only writer.
///
/// [`LatestStore`]: crate::calibrations::LatestStore
pub(crate) struct LatestStoreImpl {
    index: File,
    contents: Dir,
    capacity: usize,
}

impl LatestStoreImpl {
    /// The store in `layout`, holding at most `capacity` keys.
    pub(crate) fn new(layout: LatestCalsLayout, capacity: usize) -> Self {
        Self {
            index: layout.index(),
            contents: layout.contents(),
            capacity,
        }
    }

    /// Make `metadata` its key's entry, with `data` as its content.
    pub(crate) async fn put(
        &self,
        metadata: ReportMetadata,
        data: &str,
    ) -> Result<(), CalibrationsErr> {
        let mut index = self.read_index().await?;
        let replaced = take_entry(&mut index, &metadata);
        if replaced.is_none() {
            check_capacity(index.len(), self.capacity)?;
        }
        let report_id = metadata.report_id;
        self.write_content(report_id, data).await?;
        index.push(metadata);
        self.write_index(index).await?;
        // the same report put again shares its content file, so keep it
        if let Some(replaced) = replaced.filter(|r| r.report_id != report_id) {
            self.delete_content(replaced.report_id).await;
        }
        Ok(())
    }

    /// The entries `filter` matches, ordered by component, role and aspect.
    pub(crate) async fn list(
        &self,
        filter: &Filter,
    ) -> Result<Vec<ReportMetadata>, CalibrationsErr> {
        let mut entries = self.read_index().await?;
        entries.retain(|metadata| filter.is_match(metadata));
        entries.sort_by(|a, b| a.key().cmp(&b.key()));
        Ok(entries)
    }

    /// The entry for `report_id`, with its data.
    pub(crate) async fn get(&self, report_id: Uuid) -> Result<Report, CalibrationsErr> {
        let index = self.read_index().await?;
        let Some(entry) = index.into_iter().find(|m| m.report_id == report_id) else {
            return Err(not_found(report_id));
        };
        match files::read_string(&self.content_file(report_id)).await {
            Err(FileSysErr::PathDoesNotExistErr(_)) => Err(not_found(report_id)),
            result => Ok(Report {
                metadata: entry,
                data: result?,
            }),
        }
    }

    /// The index, which is empty until the first put writes it.
    async fn read_index(&self) -> Result<Index, CalibrationsErr> {
        match files::read_json_with_retry::<IndexFile>(&self.index).await {
            Err(FileSysErr::PathDoesNotExistErr(_)) => Ok(Index::new()),
            result => Ok(result?.entries),
        }
    }

    async fn write_index(&self, index: Index) -> Result<(), FileSysErr> {
        let file = IndexFile { entries: index };
        files::write_json(&self.index, &file, WriteOptions::OVERWRITE_ATOMIC).await
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

    /// Delete the content no entry names, which a crash or a failed write or
    /// delete leaves. If the index can't be read, nothing is deleted.
    pub(crate) async fn prune_orphaned_contents(&self) {
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
        .position(|entry| entry.key() == metadata.key())?;
    Some(index.swap_remove(slot))
}

/// Whether `value` is one of `values`, where an empty list matches any value.
fn matches<T: PartialEq>(values: &[T], value: &T) -> bool {
    values.is_empty() || values.contains(value)
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
    use super::{check_capacity, matches, not_found, take_entry, Index};
    use crate::test_utils::calibrations::reported;
    use miru_agent::calibrations::ReportMetadata;
    use miru_agent::models::InstanceFormat;

    // external crates
    use chrono::TimeDelta;
    use uuid::Uuid;

    /// Another report for `metadata`'s key, differing in every other field.
    fn same_key(metadata: &ReportMetadata) -> ReportMetadata {
        ReportMetadata {
            report_id: Uuid::new_v4(),
            format: InstanceFormat::Yaml,
            serial_number: Some("other".to_string()),
            reported_by: Some("other".to_string()),
            deployment_id: Some("other".to_string()),
            reported_at: metadata.reported_at + TimeDelta::seconds(1),
            ..metadata.clone()
        }
    }

    // ============================ TAKE ENTRY ============================ //
    #[test]
    fn take_entry_removes_the_entry_with_the_same_key() {
        let arm = reported("arm", None, None);
        // shares the component, and comes first
        let left_arm = reported("arm", Some("left"), None);
        let mut index: Index = vec![left_arm.clone(), arm.clone()];

        let taken = take_entry(&mut index, &same_key(&arm));

        assert_eq!(taken, Some(arm));
        assert_eq!(index, [left_arm]);
    }

    #[test]
    fn take_entry_leaves_the_index_for_a_new_key() {
        let arm = reported("arm", None, None);
        let mut index: Index = vec![arm.clone()];

        let taken = take_entry(&mut index, &reported("leg", None, None));

        assert_eq!(taken, None);
        assert_eq!(index, [arm]);
    }

    // =============================== KEY ================================ //
    #[test]
    fn key_is_the_component_role_and_aspect_only() {
        let arm = reported("arm", Some("left"), Some("pose"));
        // each differs from `arm` in one part of the key
        let others = [
            reported("leg", Some("left"), Some("pose")),
            reported("arm", Some("right"), Some("pose")),
            reported("arm", Some("left"), Some("offset")),
        ];

        assert_eq!(arm.key(), same_key(&arm).key());
        for other in &others {
            assert_ne!(arm.key(), other.key(), "{other:?}");
        }
    }

    // ============================== MATCHES ============================= //
    #[test]
    fn matches_listed_values_or_any_for_an_empty_list() {
        type Slug = Option<&'static str>;
        let cases: [(&[Slug], Slug, bool); 5] = [
            (&[], Some("left"), true),
            (&[Some("left"), Some("right")], Some("left"), true),
            (&[Some("right")], Some("left"), false),
            // `None` is a value too, matching only an entry without one
            (&[None], None, true),
            (&[None], Some("left"), false),
        ];

        for (values, value, want) in cases {
            assert_eq!(matches(values, &value), want, "{values:?} {value:?}");
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
            assert_eq!(result.is_ok(), accepted, "{size}/{capacity}");
        }
    }

    #[test]
    fn check_capacity_names_the_capacity() {
        // more keys than the cap, so the error can't take its count from them
        let err = check_capacity(2, 1).unwrap_err();

        assert_eq!(
            err.to_string(),
            "latest calibrations store is full (1 keys)"
        );
    }

    // ============================ NOT FOUND ============================= //
    #[test]
    fn not_found_names_the_report() {
        let report_id = Uuid::new_v4();

        let err = not_found(report_id);

        let want = format!("no latest calibration has report id {report_id}");
        assert_eq!(err.to_string(), want);
    }
}
