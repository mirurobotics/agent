//! The latest calibrations store: its keys and capacity, write order, upload
//! marks, listing and content over the calibration caches, plus the on-disk
//! wire pin for `resources/calibrations/index.json`.

// standard crates
use std::collections::HashSet;

// internal crates
use crate::test_utils::calibrations::{calibration_stor, calibrations_layout, report, report_id};
use crate::test_utils::filesys::dirs as test_dirs;
use miru_agent::calibrations::{
    latest::DEFAULT_CAPACITY, CalibrationsErr, LatestCalibrations, LatestContent, LatestEntry,
    LatestFilter, LatestStore, ListedEntry, ReportMetadata,
};
use miru_agent::disk::{layout::CalibrationsLayout, CalibrationStor};
use miru_agent::filesys::{dirs, files, File, Overwrite, PathExt, WriteOptions};
use miru_agent::models::{CalibrationUploadStatus, InstanceFormat};

// external crates
use chrono::DateTime;
use uuid::Uuid;

/// The content every fixture report is put with.
const DATA: &str = "offset: 1\n";

struct Fixture {
    dir: test_dirs::TempDir,
    layout: CalibrationsLayout,
    stor: CalibrationStor,
}

impl Fixture {
    async fn new() -> Self {
        let dir = test_dirs::temp("latest_calibrations_test").unwrap();
        let layout = calibrations_layout(&dir);
        let stor = calibration_stor(&dir).await;
        Self { dir, layout, stor }
    }

    fn store(&self) -> LatestStore {
        self.store_with(DEFAULT_CAPACITY)
    }

    fn store_with(&self, capacity: usize) -> LatestStore {
        LatestStore::new(self.stor.clone(), capacity)
    }

    /// A store on new caches over the fixture's files, as the next boot opens
    /// them.
    async fn reopened(&self) -> LatestStore {
        LatestStore::new(calibration_stor(&self.dir).await, DEFAULT_CAPACITY)
    }

    /// The metadata cache's entries, in key order.
    async fn entries(&self) -> Vec<LatestEntry> {
        let mut entries = self.stor.meta.values().await.unwrap();
        entries.sort_by_key(|e| key_parts(&e.metadata));
        entries
    }

    /// The `report_id` of each content in the content cache, sorted.
    async fn content_ids(&self) -> Vec<Uuid> {
        let keys = self.stor.content.entry_map().await.unwrap().into_keys();
        let mut ids: Vec<Uuid> = keys.map(|key| key.parse().unwrap()).collect();
        ids.sort();
        ids
    }

    /// The content cache's file for `report_id`.
    fn content_file(&self, report_id: Uuid) -> File {
        self.layout.contents().file(&format!("{report_id}.json"))
    }

    /// Put a directory where `report_id`'s content file is, so it can be
    /// neither read nor deleted.
    async fn block_content(&self, report_id: Uuid) {
        let file = self.content_file(report_id);
        files::delete(&file).await.unwrap();
        let blocker = self.layout.contents().subdir(file.name().unwrap());
        dirs::create(&blocker).await.unwrap();
    }
}

fn key_parts(metadata: &ReportMetadata) -> (String, Option<String>, Option<String>) {
    let ReportMetadata {
        component,
        role,
        aspect,
        ..
    } = metadata.clone();
    (component, role, aspect)
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
async fn put_all(store: &LatestStore, reports: &[&ReportMetadata]) {
    for &metadata in reports {
        store.put(metadata.clone(), DATA).await.unwrap();
    }
}

/// Every entry, with no report still queued.
async fn list_all(store: &LatestStore) -> Vec<ListedEntry> {
    let latest = store.list(&LatestFilter::default(), &HashSet::new()).await;
    latest.unwrap().entries
}

/// Lock the calibrations directory, so the metadata cache can be read but
/// not written, while the contents directory stays writable.
#[cfg(unix)]
async fn lock_index(f: &Fixture, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let permissions = std::fs::Permissions::from_mode(mode);
    dirs::set_permissions(&f.layout.root, permissions)
        .await
        .unwrap();
}

#[tokio::test]
async fn the_entries_survive_a_reopen() {
    let f = Fixture::new().await;
    let store = f.store();
    let arm = reported("arm", None, None);
    put_all(&store, &[&arm]).await;
    store.mark_uploaded(arm.report_id).await;

    let reopened = f.reopened().await;

    let uploaded = listed(&arm, CalibrationUploadStatus::Uploaded);
    assert_eq!(list_all(&reopened).await, [uploaded]);
}

// =============================== CAPACITY =============================== //
#[tokio::test]
async fn verify_capacity_allows_a_new_key_below_capacity() {
    let f = Fixture::new().await;
    let store = f.store_with(1);

    let result = store.verify_capacity(&reported("arm", None, None)).await;

    assert!(result.is_ok(), "{result:?}");
}

#[tokio::test]
async fn verify_capacity_rejects_a_new_key_at_capacity() {
    let f = Fixture::new().await;
    let store = f.store_with(1);
    put_all(&store, &[&reported("arm", None, None)]).await;

    let result = store.verify_capacity(&reported("leg", None, None)).await;

    let err = result.unwrap_err();
    assert!(matches!(err, CalibrationsErr::StoreFullErr(_)), "{err:?}");
}

#[tokio::test]
async fn verify_capacity_allows_an_existing_key_at_capacity() {
    let f = Fixture::new().await;
    let store = f.store_with(1);
    let old = reported("arm", None, None);
    put_all(&store, &[&old]).await;

    let result = store.verify_capacity(&newer(&old)).await;

    assert!(result.is_ok(), "{result:?}");
}

// ================================= PUT ================================== //
#[tokio::test]
async fn put_adds_an_entry_not_uploaded_and_its_content() {
    let f = Fixture::new().await;
    let store = f.store();
    let arm = reported("arm", None, None);

    store.put(arm.clone(), DATA).await.unwrap();

    assert_eq!(f.entries().await, [entry(&arm, false)]);
    assert_eq!(f.content_ids().await, [arm.report_id]);
}

#[tokio::test]
async fn put_replaces_the_keys_entry_and_deletes_its_content() {
    let f = Fixture::new().await;
    let store = f.store();
    let old = reported("arm", None, None);
    let new = newer(&old);
    put_all(&store, &[&old]).await;

    store.put(new.clone(), DATA).await.unwrap();

    assert_eq!(f.entries().await, [entry(&new, false)]);
    assert_eq!(f.content_ids().await, [new.report_id]);
}

#[tokio::test]
async fn role_and_aspect_are_part_of_the_key() {
    let f = Fixture::new().await;
    let store = f.store();
    let bare = reported("arm", None, None);
    let with_role = reported("arm", Some("left"), None);
    let with_aspect = reported("arm", None, Some("pose"));

    put_all(&store, &[&bare, &with_role, &with_aspect]).await;

    // in key order, a missing slug first
    let want = [&bare, &with_aspect, &with_role].map(|m| entry(m, false));
    assert_eq!(f.entries().await, want);
}

#[tokio::test]
async fn put_fails_and_changes_nothing_when_the_content_cannot_be_written() {
    let f = Fixture::new().await;
    let store = f.store();
    // a regular file where the contents directory should be
    let contents = f.layout.contents();
    dirs::delete(&contents).await.unwrap();
    let blocker = File::new(contents.path().clone());
    files::write_string(&blocker, "x", WriteOptions::OVERWRITE_ATOMIC)
        .await
        .unwrap();

    let arm = reported("arm", None, None);

    let err = store.put(arm, DATA).await.unwrap_err();

    assert!(matches!(err, CalibrationsErr::CacheErr(_)), "{err:?}");
    assert!(f.entries().await.is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn put_fails_and_changes_nothing_when_the_entry_cannot_be_written() {
    let f = Fixture::new().await;
    let store = f.store();
    let old = reported("arm", None, None);
    put_all(&store, &[&old]).await;
    lock_index(&f, 0o555).await;

    let result = store.put(newer(&old), DATA).await;

    lock_index(&f, 0o755).await;
    let err = result.unwrap_err();
    assert!(matches!(err, CalibrationsErr::CacheErr(_)), "{err:?}");
    assert_eq!(f.entries().await, [entry(&old, false)]);
    // the new report's content is deleted again
    assert_eq!(f.content_ids().await, [old.report_id]);
}

#[tokio::test]
async fn put_succeeds_when_the_replaced_content_cannot_be_deleted() {
    let f = Fixture::new().await;
    let store = f.store();
    let old = reported("arm", None, None);
    let new = newer(&old);
    put_all(&store, &[&old]).await;
    f.block_content(old.report_id).await;

    store.put(new.clone(), DATA).await.unwrap();

    assert_eq!(f.entries().await, [entry(&new, false)]);
}

#[tokio::test]
async fn a_corrupt_index_fails_every_call() {
    let f = Fixture::new().await;
    let store = f.store();
    files::write_string(&f.layout.index(), "x", WriteOptions::OVERWRITE_ATOMIC)
        .await
        .unwrap();
    let arm = reported("arm", None, None);

    let errs = [
        store.verify_capacity(&arm).await.unwrap_err(),
        store.put(arm.clone(), DATA).await.unwrap_err(),
        store
            .list(&LatestFilter::default(), &HashSet::new())
            .await
            .unwrap_err(),
        store.content(arm.report_id).await.unwrap_err(),
    ];

    for err in errs {
        assert!(matches!(err, CalibrationsErr::CacheErr(_)), "{err:?}");
    }
    // a mark or prune it can't read is logged, not raised
    store.mark_uploaded(arm.report_id).await;
    store.prune_orphans().await;
}

// ============================ MARK UPLOADED ============================= //
#[tokio::test]
async fn mark_uploaded_marks_the_matching_report() {
    let f = Fixture::new().await;
    let store = f.store();
    let arm = reported("arm", None, None);
    let leg = reported("leg", None, None);
    put_all(&store, &[&arm, &leg]).await;

    store.mark_uploaded(arm.report_id).await;

    assert_eq!(f.entries().await, [entry(&arm, true), entry(&leg, false)]);
}

#[tokio::test]
async fn mark_uploaded_ignores_a_replaced_report() {
    let f = Fixture::new().await;
    let store = f.store();
    let old = reported("arm", None, None);
    let new = newer(&old);
    put_all(&store, &[&old, &new]).await;

    store.mark_uploaded(old.report_id).await;

    assert_eq!(f.entries().await, [entry(&new, false)]);
}

#[cfg(unix)]
#[tokio::test]
async fn a_mark_that_cannot_be_written_leaves_the_entry() {
    let f = Fixture::new().await;
    let store = f.store();
    let arm = reported("arm", None, None);
    put_all(&store, &[&arm]).await;
    lock_index(&f, 0o555).await;

    store.mark_uploaded(arm.report_id).await;

    lock_index(&f, 0o755).await;
    assert_eq!(f.entries().await, [entry(&arm, false)]);
}

// ================================= LIST ================================= //
#[tokio::test]
async fn list_derives_the_upload_status() {
    let f = Fixture::new().await;
    let store = f.store();
    let uploaded = reported("a", None, None);
    let queued = reported("b", None, None);
    let dropped = reported("c", None, None);
    put_all(&store, &[&uploaded, &queued, &dropped]).await;
    store.mark_uploaded(uploaded.report_id).await;
    // a report is marked uploaded before it leaves the queue
    let in_queue = HashSet::from([uploaded.report_id, queued.report_id]);

    let latest = store.list(&LatestFilter::default(), &in_queue).await;

    let want = vec![
        listed(&uploaded, CalibrationUploadStatus::Uploaded),
        listed(&queued, CalibrationUploadStatus::Queued),
        listed(&dropped, CalibrationUploadStatus::Dropped),
    ];
    assert_eq!(latest.unwrap().entries, want);
}

#[tokio::test]
async fn an_empty_filter_lists_every_entry_unconfirmed() {
    let f = Fixture::new().await;
    let store = f.store();
    let arm = reported("arm", Some("left"), Some("pose"));
    let leg = reported("leg", None, None);
    put_all(&store, &[&arm, &leg]).await;

    let latest = store.list(&LatestFilter::default(), &HashSet::new()).await;

    let want = LatestCalibrations {
        is_confirmed: false,
        entries: vec![
            listed(&arm, CalibrationUploadStatus::Dropped),
            listed(&leg, CalibrationUploadStatus::Dropped),
        ],
    };
    assert_eq!(latest.unwrap(), want);
}

#[tokio::test]
async fn list_filters_by_component_role_and_aspect() {
    let f = Fixture::new().await;
    let store = f.store();
    let matched = reported("arm", Some("left"), Some("pose"));
    // each misses the filter on one slug
    let other_component = reported("leg", Some("left"), Some("pose"));
    let other_role = reported("arm", Some("right"), Some("pose"));
    let other_aspect = reported("arm", Some("left"), Some("offset"));
    let reports = [&matched, &other_component, &other_role, &other_aspect];
    put_all(&store, &reports).await;
    let filter = LatestFilter {
        component: vec!["arm".to_string(), "wrist".to_string()],
        role: vec![Some("left".to_string())],
        aspect: vec![Some("pose".to_string())],
    };

    let latest = store.list(&filter, &HashSet::new()).await;

    let want = [listed(&matched, CalibrationUploadStatus::Dropped)];
    assert_eq!(latest.unwrap().entries, want);
}

#[tokio::test]
async fn list_matches_null_role_and_aspect() {
    let f = Fixture::new().await;
    let store = f.store();
    let bare = reported("arm", None, None);
    let with_role = reported("arm", Some("left"), None);
    let with_aspect = reported("arm", None, Some("pose"));
    put_all(&store, &[&bare, &with_role, &with_aspect]).await;
    let filter = LatestFilter {
        role: vec![None],
        aspect: vec![None],
        ..LatestFilter::default()
    };

    let latest = store.list(&filter, &HashSet::new()).await;

    let want = [listed(&bare, CalibrationUploadStatus::Dropped)];
    assert_eq!(latest.unwrap().entries, want);
}

// ============================ PRUNE ORPHANS ============================= //
#[tokio::test]
async fn prune_orphans_deletes_the_content_no_entry_names() {
    let f = Fixture::new().await;
    let store = f.store();
    let arm = reported("arm", None, None);
    put_all(&store, &[&arm]).await;
    let orphan = Uuid::new_v4().to_string();
    let content = &f.stor.content;
    content
        .write(orphan, DATA.to_string(), |_, _| false, Overwrite::Allow)
        .await
        .unwrap();

    store.prune_orphans().await;

    assert_eq!(f.content_ids().await, [arm.report_id]);
}

#[tokio::test]
async fn a_prune_that_cannot_list_the_contents_keeps_the_entries() {
    let f = Fixture::new().await;
    let store = f.store();
    let arm = reported("arm", None, None);
    put_all(&store, &[&arm]).await;
    // a regular file where the contents directory should be
    let contents = f.layout.contents();
    dirs::delete(&contents).await.unwrap();
    let blocker = File::new(contents.path().clone());
    files::write_string(&blocker, "x", WriteOptions::OVERWRITE_ATOMIC)
        .await
        .unwrap();

    store.prune_orphans().await;

    assert_eq!(f.entries().await, [entry(&arm, false)]);
}

// ================================ CONTENT =============================== //
#[tokio::test]
async fn content_returns_the_bytes_and_format() {
    let f = Fixture::new().await;
    let store = f.store();
    let arm = ReportMetadata {
        format: InstanceFormat::Yaml,
        ..reported("arm", None, None)
    };
    put_all(&store, &[&arm]).await;

    let content = store.content(arm.report_id).await.unwrap();

    let want = LatestContent {
        format: InstanceFormat::Yaml,
        data: DATA.as_bytes().to_vec(),
    };
    assert_eq!(content, want);
}

#[tokio::test]
async fn content_of_an_unknown_report_is_not_found() {
    let f = Fixture::new().await;
    let store = f.store();
    put_all(&store, &[&reported("arm", None, None)]).await;

    let err = store.content(Uuid::new_v4()).await.unwrap_err();

    assert!(matches!(err, CalibrationsErr::NotFoundErr(_)), "{err:?}");
}

#[tokio::test]
async fn content_whose_file_is_missing_is_not_found() {
    let f = Fixture::new().await;
    let store = f.store();
    let arm = reported("arm", None, None);
    put_all(&store, &[&arm]).await;
    files::delete(&f.content_file(arm.report_id)).await.unwrap();

    let err = store.content(arm.report_id).await.unwrap_err();

    assert!(matches!(err, CalibrationsErr::NotFoundErr(_)), "{err:?}");
}

#[tokio::test]
async fn an_unreadable_content_is_a_cache_error() {
    let f = Fixture::new().await;
    let store = f.store();
    let arm = reported("arm", None, None);
    put_all(&store, &[&arm]).await;
    f.block_content(arm.report_id).await;

    let err = store.content(arm.report_id).await.unwrap_err();

    assert!(matches!(err, CalibrationsErr::CacheErr(_)), "{err:?}");
}

mod wire {
    use super::*;

    /// Pins the persisted shape of `index.json`, keys included, so a shape
    /// change fails here rather than emptying the store of a running agent.
    #[tokio::test]
    async fn raw_json_index_loads() {
        let f = Fixture::new().await;
        let raw = format!(
            concat!(
                r#"{{"[\"arm\",\"aligner-1\",\"pose\"]":{{"#,
                r#""key":"[\"arm\",\"aligner-1\",\"pose\"]","value":{{"metadata":{{"#,
                r#""report_id":"{report_id}","component":"arm","format":"yaml","#,
                r#""role":"aligner-1","aspect":"pose","serial_number":"SN-1","#,
                r#""reported_by":"Ada Lovelace","#,
                r#""deployment_id":"dpl_1","reported_at":"1970-01-01T00:16:40Z""#,
                r#"}},"is_uploaded":true}},"is_dirty":false,"#,
                r#""created_at":"1970-01-01T00:16:40Z","last_accessed":"1970-01-01T00:16:40Z""#,
                r#"}}}}"#,
            ),
            report_id = report_id("arm"),
        );
        files::write_string(&f.layout.index(), &raw, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();
        let pinned = ReportMetadata {
            report_id: report_id("arm"),
            component: "arm".to_string(),
            format: InstanceFormat::Yaml,
            role: Some("aligner-1".to_string()),
            aspect: Some("pose".to_string()),
            serial_number: Some("SN-1".to_string()),
            reported_by: Some("Ada Lovelace".to_string()),
            deployment_id: Some("dpl_1".to_string()),
            reported_at: DateTime::from_timestamp(1000, 0).unwrap(),
        };

        let entries = list_all(&f.store()).await;

        let want = [listed(&pinned, CalibrationUploadStatus::Uploaded)];
        assert_eq!(entries, want);
        // a newer report for the same key replaces it rather than adding one
        put_all(&f.store(), &[&newer(&pinned)]).await;
        assert_eq!(f.entries().await.len(), 1);
    }
}
