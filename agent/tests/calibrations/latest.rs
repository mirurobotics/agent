//! The latest calibrations store: its keys and capacity, write order, upload
//! marks, listing, content and pruning, plus the on-disk wire pin for
//! `resources/calibrations/index.json`.

// standard crates
use std::collections::HashSet;

// internal crates
use crate::test_utils::calibrations::{latest_layout, report, report_id};
use crate::test_utils::filesys::dirs as test_dirs;
use miru_agent::calibrations::{
    latest::DEFAULT_CAPACITY, CalibrationsErr, LatestCalibrations, LatestContent, LatestEntry,
    LatestFilter, LatestIndex, LatestIndexFile, LatestStore, ListedEntry, ReportMetadata,
};
use miru_agent::disk::layout::LatestCalibrationsLayout;
use miru_agent::filesys::{dirs, files, File, PathExt, WriteOptions};
use miru_agent::models::{CalibrationUploadStatus, InstanceFormat};

// external crates
use chrono::DateTime;
use uuid::Uuid;

/// The content every fixture report is put with.
const DATA: &[u8] = b"offset: 1\n";

struct Fixture {
    _dir: test_dirs::TempDir,
    layout: LatestCalibrationsLayout,
}

impl Fixture {
    fn new() -> Self {
        let dir = test_dirs::temp("latest_calibrations_test").unwrap();
        let layout = latest_layout(&dir);
        Self { _dir: dir, layout }
    }

    async fn index_file(&self) -> LatestIndexFile {
        LatestIndexFile::new_with_default(self.layout.index(), Default::default())
            .await
            .unwrap()
    }

    /// A store saved under the fixture's directory.
    async fn store(&self) -> LatestStore {
        self.store_with(DEFAULT_CAPACITY).await
    }

    async fn store_with(&self, capacity: usize) -> LatestStore {
        let index_file = Some(self.index_file().await);
        LatestStore::new(index_file, self.layout.contents(), capacity)
    }

    /// The index file's entries, as the next boot would load them.
    async fn on_disk(&self) -> Vec<LatestEntry> {
        let index = self.index_file().await.read();
        index.entries.values().cloned().collect()
    }

    fn content_file(&self, report_id: Uuid) -> File {
        self.layout.contents().file(&report_id.to_string())
    }

    async fn contents(&self) -> Vec<File> {
        dirs::files(&self.layout.contents()).await.unwrap()
    }

    /// Make the index unwritable: a non-empty directory where the index file
    /// should be can't be replaced.
    async fn block_index(&self) {
        files::delete(&self.layout.index()).await.unwrap();
        let blocker = self.layout.root.subdir("index.json");
        files::write_string(&blocker.file("x"), "x", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();
    }
}

/// A report for the key (`component`, `role`, `aspect`) with its own
/// `report_id`.
fn reported(component: &str, role: Option<&str>, aspect: Option<&str>) -> ReportMetadata {
    ReportMetadata {
        report_id: Uuid::new_v4(),
        role: role.map(str::to_string),
        aspect: aspect.map(str::to_string),
        ..report(component).metadata
    }
}

/// A newer report for `metadata`'s key, with a fresh `report_id` as intake
/// mints.
fn newer(metadata: &ReportMetadata) -> ReportMetadata {
    ReportMetadata {
        report_id: Uuid::new_v4(),
        ..metadata.clone()
    }
}

fn entry(metadata: &ReportMetadata, is_uploaded: bool) -> LatestEntry {
    LatestEntry {
        metadata: metadata.clone(),
        is_uploaded,
    }
}

fn listed(metadata: &ReportMetadata, upload_status: CalibrationUploadStatus) -> ListedEntry {
    ListedEntry {
        metadata: metadata.clone(),
        upload_status,
    }
}

/// Put each report with [`DATA`], in order.
async fn put_all(store: &mut LatestStore, reports: &[&ReportMetadata]) {
    for &metadata in reports {
        store.put(metadata.clone(), DATA).await.unwrap();
    }
}

/// Every entry, with no report still queued.
fn list_all(store: &LatestStore) -> Vec<ListedEntry> {
    let latest = store.list(&LatestFilter::default(), &HashSet::new());
    latest.entries
}

// ================================= NEW ================================== //
#[tokio::test]
async fn the_index_survives_a_reopen() {
    let f = Fixture::new();
    let mut store = f.store().await;
    let arm = reported("arm", None, None);
    put_all(&mut store, &[&arm]).await;
    store.mark_uploaded(arm.report_id).await;

    let reopened = f.store().await;

    assert_eq!(
        list_all(&reopened),
        [listed(&arm, CalibrationUploadStatus::Uploaded)]
    );
}

// ================================= PUT ================================== //
#[tokio::test]
async fn put_adds_an_entry_not_uploaded_and_its_content() {
    let f = Fixture::new();
    let mut store = f.store().await;
    let arm = reported("arm", None, None);

    store.put(arm.clone(), DATA).await.unwrap();

    assert_eq!(f.on_disk().await, [entry(&arm, false)]);
    let file = f.content_file(arm.report_id);
    assert_eq!(files::read_bytes(&file).await.unwrap(), DATA);
}

#[tokio::test]
async fn put_replaces_the_keys_entry_and_deletes_its_content() {
    let f = Fixture::new();
    let mut store = f.store().await;
    let old = reported("arm", None, None);
    let new = newer(&old);
    put_all(&mut store, &[&old]).await;

    store.put(new.clone(), DATA).await.unwrap();

    assert_eq!(f.on_disk().await, [entry(&new, false)]);
    assert_eq!(f.contents().await, [f.content_file(new.report_id)]);
}

#[tokio::test]
async fn role_and_aspect_are_part_of_the_key() {
    let f = Fixture::new();
    let mut store = f.store().await;
    let bare = reported("arm", None, None);
    let with_role = reported("arm", Some("left"), None);
    let with_aspect = reported("arm", None, Some("pose"));
    let reports = [&bare, &with_role, &with_aspect];

    put_all(&mut store, &reports).await;

    // in key order, a missing slug first
    let want = [&bare, &with_aspect, &with_role].map(|m| entry(m, false));
    assert_eq!(f.on_disk().await, want);
}

#[tokio::test]
async fn put_rejects_a_new_key_at_capacity_and_writes_nothing() {
    let f = Fixture::new();
    let mut store = f.store_with(1).await;
    let arm = reported("arm", None, None);
    let leg = reported("leg", None, None);
    put_all(&mut store, &[&arm]).await;

    let err = store.put(leg, DATA).await.unwrap_err();

    assert!(matches!(err, CalibrationsErr::StoreFullErr(_)), "{err:?}");
    assert_eq!(f.on_disk().await, [entry(&arm, false)]);
    assert_eq!(f.contents().await, [f.content_file(arm.report_id)]);
}

#[tokio::test]
async fn put_replaces_an_existing_key_at_capacity() {
    let f = Fixture::new();
    let mut store = f.store_with(1).await;
    let old = reported("arm", None, None);
    let new = newer(&old);
    put_all(&mut store, &[&old]).await;

    store.put(new.clone(), DATA).await.unwrap();

    assert_eq!(f.on_disk().await, [entry(&new, false)]);
}

#[tokio::test]
async fn put_fails_and_changes_nothing_when_the_content_cannot_be_written() {
    let f = Fixture::new();
    let mut store = f.store().await;
    // a regular file where the contents directory should be
    let blocker = File::new(f.layout.contents().path().clone());
    files::write_string(&blocker, "x", WriteOptions::OVERWRITE_ATOMIC)
        .await
        .unwrap();
    let arm = reported("arm", None, None);

    let err = store.put(arm, DATA).await.unwrap_err();

    assert!(matches!(err, CalibrationsErr::FileSysErr(_)), "{err:?}");
    assert!(list_all(&store).is_empty());
    assert!(f.on_disk().await.is_empty());
}

#[tokio::test]
async fn put_fails_and_changes_nothing_when_the_index_cannot_be_written() {
    let f = Fixture::new();
    let mut store = f.store().await;
    let old = reported("arm", None, None);
    put_all(&mut store, &[&old]).await;
    f.block_index().await;

    let err = store.put(newer(&old), DATA).await.unwrap_err();

    assert!(matches!(err, CalibrationsErr::FileSysErr(_)), "{err:?}");
    assert_eq!(
        list_all(&store),
        [listed(&old, CalibrationUploadStatus::Dropped)]
    );
    // the new report's content file is deleted again
    assert_eq!(f.contents().await, [f.content_file(old.report_id)]);
}

// ============================ MARK UPLOADED ============================= //
#[tokio::test]
async fn mark_uploaded_marks_the_matching_report() {
    let f = Fixture::new();
    let mut store = f.store().await;
    let arm = reported("arm", None, None);
    let leg = reported("leg", None, None);
    put_all(&mut store, &[&arm, &leg]).await;

    store.mark_uploaded(arm.report_id).await;

    assert_eq!(f.on_disk().await, [entry(&arm, true), entry(&leg, false)]);
}

#[tokio::test]
async fn mark_uploaded_ignores_a_replaced_report() {
    let f = Fixture::new();
    let mut store = f.store().await;
    let old = reported("arm", None, None);
    let new = newer(&old);
    put_all(&mut store, &[&old, &new]).await;

    store.mark_uploaded(old.report_id).await;

    assert_eq!(f.on_disk().await, [entry(&new, false)]);
}

#[tokio::test]
async fn a_mark_the_index_cannot_save_is_kept_in_memory() {
    let f = Fixture::new();
    let mut store = f.store().await;
    let arm = reported("arm", None, None);
    put_all(&mut store, &[&arm]).await;
    f.block_index().await;

    store.mark_uploaded(arm.report_id).await;

    assert_eq!(
        list_all(&store),
        [listed(&arm, CalibrationUploadStatus::Uploaded)]
    );
}

// ================================= LIST ================================= //
#[tokio::test]
async fn list_derives_the_upload_status() {
    let f = Fixture::new();
    let mut store = f.store().await;
    let uploaded = reported("a", None, None);
    let queued = reported("b", None, None);
    let dropped = reported("c", None, None);
    put_all(&mut store, &[&uploaded, &queued, &dropped]).await;
    store.mark_uploaded(uploaded.report_id).await;
    // a report is marked uploaded before it leaves the queue
    let in_queue = HashSet::from([uploaded.report_id, queued.report_id]);

    let latest = store.list(&LatestFilter::default(), &in_queue);

    let want = vec![
        listed(&uploaded, CalibrationUploadStatus::Uploaded),
        listed(&queued, CalibrationUploadStatus::Queued),
        listed(&dropped, CalibrationUploadStatus::Dropped),
    ];
    assert_eq!(latest.entries, want);
}

#[tokio::test]
async fn an_empty_filter_lists_every_entry() {
    let f = Fixture::new();
    let mut store = f.store().await;
    let arm = reported("arm", Some("left"), Some("pose"));
    let leg = reported("leg", None, None);
    put_all(&mut store, &[&arm, &leg]).await;

    let latest = store.list(&LatestFilter::default(), &HashSet::new());

    let want = LatestCalibrations {
        is_confirmed: false,
        entries: vec![
            listed(&arm, CalibrationUploadStatus::Dropped),
            listed(&leg, CalibrationUploadStatus::Dropped),
        ],
    };
    assert_eq!(latest, want);
}

#[tokio::test]
async fn list_filters_by_component_role_and_aspect() {
    let f = Fixture::new();
    let mut store = f.store().await;
    let matched = reported("arm", Some("left"), Some("pose"));
    // each misses the filter on one slug
    let other_component = reported("leg", Some("left"), Some("pose"));
    let other_role = reported("arm", Some("right"), Some("pose"));
    let other_aspect = reported("arm", Some("left"), Some("offset"));
    let reports = [&matched, &other_component, &other_role, &other_aspect];
    put_all(&mut store, &reports).await;
    let filter = LatestFilter {
        component: vec!["arm".to_string(), "wrist".to_string()],
        role: vec![Some("left".to_string())],
        aspect: vec![Some("pose".to_string())],
    };

    let latest = store.list(&filter, &HashSet::new());

    assert_eq!(
        latest.entries,
        [listed(&matched, CalibrationUploadStatus::Dropped)]
    );
}

#[tokio::test]
async fn list_matches_null_role_and_aspect() {
    let f = Fixture::new();
    let mut store = f.store().await;
    let bare = reported("arm", None, None);
    let with_role = reported("arm", Some("left"), None);
    let with_aspect = reported("arm", None, Some("pose"));
    let reports = [&bare, &with_role, &with_aspect];
    put_all(&mut store, &reports).await;
    let filter = LatestFilter {
        role: vec![None],
        aspect: vec![None],
        ..LatestFilter::default()
    };

    let latest = store.list(&filter, &HashSet::new());

    assert_eq!(
        latest.entries,
        [listed(&bare, CalibrationUploadStatus::Dropped)]
    );
}

#[tokio::test]
async fn is_confirmed_is_whether_loaded_at_is_set() {
    let f = Fixture::new();
    let unloaded = f.store().await;
    let mut index_file = f.index_file().await;
    let index = LatestIndex {
        loaded_at: DateTime::from_timestamp(2000, 0),
        ..LatestIndex::default()
    };
    index_file.patch(index).await.unwrap();

    let loaded = f.store().await;

    let (filter, none_queued) = (LatestFilter::default(), HashSet::new());
    assert!(!unloaded.list(&filter, &none_queued).is_confirmed);
    assert!(loaded.list(&filter, &none_queued).is_confirmed);
}

// ================================ CONTENT =============================== //
#[tokio::test]
async fn content_returns_the_bytes_and_format() {
    let f = Fixture::new();
    let mut store = f.store().await;
    let arm = ReportMetadata {
        format: InstanceFormat::Yaml,
        ..reported("arm", None, None)
    };
    put_all(&mut store, &[&arm]).await;

    let content = store.content(arm.report_id).await.unwrap();

    let want = LatestContent {
        format: InstanceFormat::Yaml,
        data: DATA.to_vec(),
    };
    assert_eq!(content, want);
}

#[tokio::test]
async fn content_of_an_unknown_report_is_not_found() {
    let f = Fixture::new();
    let mut store = f.store().await;
    put_all(&mut store, &[&reported("arm", None, None)]).await;

    let err = store.content(Uuid::new_v4()).await.unwrap_err();

    assert!(matches!(err, CalibrationsErr::NotFoundErr(_)), "{err:?}");
}

#[tokio::test]
async fn content_whose_file_is_missing_is_not_found() {
    let f = Fixture::new();
    let mut store = f.store().await;
    let arm = reported("arm", None, None);
    put_all(&mut store, &[&arm]).await;
    let file = f.content_file(arm.report_id);
    files::delete(&file).await.unwrap();

    let err = store.content(arm.report_id).await.unwrap_err();

    assert!(matches!(err, CalibrationsErr::NotFoundErr(_)), "{err:?}");
}

#[tokio::test]
async fn an_unreadable_content_file_is_a_file_error() {
    let f = Fixture::new();
    let mut store = f.store().await;
    let arm = reported("arm", None, None);
    put_all(&mut store, &[&arm]).await;
    // a directory where the content file should be cannot be read
    let file = f.content_file(arm.report_id);
    files::delete(&file).await.unwrap();
    let blocker = f.layout.contents().subdir(arm.report_id.to_string());
    dirs::create(&blocker).await.unwrap();

    let err = store.content(arm.report_id).await.unwrap_err();

    assert!(matches!(err, CalibrationsErr::FileSysErr(_)), "{err:?}");
}

// ============================ PRUNE ORPHANS ============================= //
#[tokio::test]
async fn prune_orphans_deletes_unreferenced_contents() {
    let f = Fixture::new();
    let mut store = f.store().await;
    let arm = reported("arm", None, None);
    put_all(&mut store, &[&arm]).await;
    let orphan = f.content_file(Uuid::new_v4());
    files::write_bytes(&orphan, DATA, WriteOptions::OVERWRITE_ATOMIC)
        .await
        .unwrap();

    store.prune_orphans().await;

    assert_eq!(f.contents().await, [f.content_file(arm.report_id)]);
}

#[tokio::test]
async fn an_in_memory_store_prunes_nothing() {
    let f = Fixture::new();
    let mut store = LatestStore::new(None, f.layout.contents(), DEFAULT_CAPACITY);
    put_all(&mut store, &[&reported("arm", None, None)]).await;
    // the index file the store couldn't open may name it
    let unnamed = f.content_file(Uuid::new_v4());
    files::write_bytes(&unnamed, DATA, WriteOptions::OVERWRITE_ATOMIC)
        .await
        .unwrap();

    store.prune_orphans().await;

    assert!(unnamed.exists());
}

mod wire {
    use super::*;

    /// The pinned report in the raw index below.
    fn pinned() -> ReportMetadata {
        ReportMetadata {
            report_id: report_id("arm"),
            component: "arm".to_string(),
            format: InstanceFormat::Yaml,
            role: Some("aligner-1".to_string()),
            aspect: Some("pose".to_string()),
            serial_number: Some("SN-1".to_string()),
            reported_by: Some("Ada Lovelace".to_string()),
            deployment_id: Some("dpl_1".to_string()),
            reported_at: DateTime::from_timestamp(1000, 0).unwrap(),
        }
    }

    /// A store on `raw` written as its index file.
    async fn load(f: &Fixture, raw: &str) -> LatestStore {
        let index = f.layout.index();
        files::write_string(&index, raw, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();
        f.store().await
    }

    /// Pins the persisted shape of `index.json`, so a shape change fails here
    /// rather than emptying the store of a running agent at its next boot.
    #[tokio::test]
    async fn raw_json_index_loads() {
        let f = Fixture::new();
        let raw = format!(
            concat!(
                r#"{{"loaded_at":"1970-01-01T00:33:20Z","entries":[{{"metadata":{{"#,
                r#""report_id":"{report_id}","component":"arm","format":"yaml","#,
                r#""role":"aligner-1","aspect":"pose","serial_number":"SN-1","#,
                r#""reported_by":"Ada Lovelace","#,
                r#""deployment_id":"dpl_1","reported_at":"1970-01-01T00:16:40Z""#,
                r#"}},"is_uploaded":true}}]}}"#,
            ),
            report_id = report_id("arm"),
        );

        let store = load(&f, &raw).await;

        let latest = store.list(&LatestFilter::default(), &HashSet::new());
        let want = LatestCalibrations {
            is_confirmed: true,
            entries: vec![listed(&pinned(), CalibrationUploadStatus::Uploaded)],
        };
        assert_eq!(latest, want);
    }

    /// Every index write includes `is_uploaded`, so an entry without it is
    /// corrupt: the index is copied aside and the store starts empty.
    #[tokio::test]
    async fn an_entry_without_is_uploaded_starts_the_store_empty() {
        let f = Fixture::new();
        let raw = format!(
            concat!(
                r#"{{"entries":[{{"metadata":{{"report_id":"{report_id}","#,
                r#""component":"arm","format":"yaml","#,
                r#""reported_at":"1970-01-01T00:16:40Z"}}}}]}}"#,
            ),
            report_id = report_id("arm"),
        );

        let store = load(&f, &raw).await;

        assert!(list_all(&store).is_empty());
    }
}
