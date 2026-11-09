//! The latest calibrations store's own rules over the calibration caches: its
//! keys and capacity, write order, upload marks, listing and content.

// standard crates
use std::collections::HashSet;

// internal crates
use crate::test_utils::calibrations::{calibration_stor, calibrations_layout, report};
use crate::test_utils::filesys::dirs as test_dirs;
use miru_agent::calibrations::{
    CalibrationsErr, LatestCalibrations, LatestContent, LatestEntry, LatestFilter, LatestStore,
    ListedEntry, ReportMetadata,
};
use miru_agent::disk::{layout::CalibrationsLayout, CalibrationStor, Capacities};
use miru_agent::filesys::{dirs, files, File, PathExt, WriteOptions};
use miru_agent::models::{CalibrationUploadStatus, InstanceFormat};

// external crates
use uuid::Uuid;

/// The content every fixture report is put with.
const DATA: &str = "offset: 1\n";

struct Fixture {
    _dir: test_dirs::TempDir,
    layout: CalibrationsLayout,
    stor: CalibrationStor,
}

impl Fixture {
    async fn new() -> Self {
        let dir = test_dirs::temp("latest_calibrations_test").unwrap();
        let layout = calibrations_layout(&dir);
        let stor = calibration_stor(&dir, Capacities::default().calibrations).await;
        Self {
            _dir: dir,
            layout,
            stor,
        }
    }

    fn store(&self) -> LatestStore {
        LatestStore::new(self.stor.clone())
    }

    /// A store over the fixture's caches that holds at most `capacity` keys.
    fn store_with(&self, capacity: usize) -> LatestStore {
        let stor = self.stor.clone();
        LatestStore::new(CalibrationStor { capacity, ..stor })
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

/// Set the mode of the calibrations directory: 0o555 lets the metadata cache
/// be read but not written, while the contents directory stays writable.
#[cfg(unix)]
async fn set_root_mode(f: &Fixture, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let permissions = std::fs::Permissions::from_mode(mode);
    dirs::set_permissions(&f.layout.root, permissions)
        .await
        .unwrap();
}

// =============================== CAPACITY =============================== //
#[tokio::test]
async fn verify_capacity_rejects_only_a_new_key_at_capacity() {
    let f = Fixture::new().await;
    let store = f.store_with(2);
    let arm = reported("arm", None, None);
    let leg = reported("leg", None, None);
    put_all(&store, &[&arm]).await;
    // one key short of full
    let below = store.verify_capacity(&leg).await;
    put_all(&store, &[&leg]).await;

    let existing_key = store.verify_capacity(&newer(&arm)).await;
    let hand = reported("hand", None, None);
    let new_key = store.verify_capacity(&hand).await;

    assert!(below.is_ok(), "{below:?}");
    assert!(existing_key.is_ok(), "{existing_key:?}");
    let err = new_key.unwrap_err();
    assert!(matches!(err, CalibrationsErr::StoreFullErr(_)), "{err:?}");
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
    let arm = reported("arm", None, None);
    // a regular file where the contents directory should be
    let contents = f.layout.contents();
    dirs::delete(&contents).await.unwrap();
    let blocker = File::new(contents.path().clone());
    files::write_string(&blocker, "x", WriteOptions::OVERWRITE_ATOMIC)
        .await
        .unwrap();

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
    set_root_mode(&f, 0o555).await;

    let result = store.put(newer(&old), DATA).await;

    set_root_mode(&f, 0o755).await;
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

// ============================ MARK UPLOADED ============================= //
#[tokio::test]
async fn mark_uploaded_marks_only_a_keys_latest_report() {
    let f = Fixture::new().await;
    let store = f.store();
    let old = reported("arm", None, None);
    let new = newer(&old);
    let leg = reported("leg", None, None);
    put_all(&store, &[&old, &new, &leg]).await;

    store.mark_uploaded(old.report_id).await;
    store.mark_uploaded(leg.report_id).await;

    assert_eq!(f.entries().await, [entry(&new, false), entry(&leg, true)]);
}

#[cfg(unix)]
#[tokio::test]
async fn a_mark_that_cannot_be_written_leaves_the_entry() {
    let f = Fixture::new().await;
    let store = f.store();
    let arm = reported("arm", None, None);
    put_all(&store, &[&arm]).await;
    set_root_mode(&f, 0o555).await;

    store.mark_uploaded(arm.report_id).await;

    set_root_mode(&f, 0o755).await;
    assert_eq!(f.entries().await, [entry(&arm, false)]);
}

// ================================= LIST ================================= //
#[tokio::test]
async fn an_empty_filter_lists_every_entry_sorted_with_its_status() {
    let f = Fixture::new().await;
    let store = f.store();
    let uploaded = reported("a", Some("left"), Some("pose"));
    let queued = reported("b", None, None);
    let dropped = reported("c", None, None);
    put_all(&store, &[&dropped, &queued, &uploaded]).await;
    store.mark_uploaded(uploaded.report_id).await;
    // a report is marked uploaded before it leaves the queue
    let in_queue = HashSet::from([uploaded.report_id, queued.report_id]);

    let latest = store.list(&LatestFilter::default(), &in_queue).await;

    let want = LatestCalibrations {
        is_confirmed: false,
        entries: vec![
            listed(&uploaded, CalibrationUploadStatus::Uploaded),
            listed(&queued, CalibrationUploadStatus::Queued),
            listed(&dropped, CalibrationUploadStatus::Dropped),
        ],
    };
    assert_eq!(latest.unwrap(), want);
}

#[tokio::test]
async fn list_filters_by_component_role_and_aspect() {
    let f = Fixture::new().await;
    let store = f.store();
    let matched = reported("arm", Some("left"), Some("pose"));
    // each misses the first filter on one slug
    let other_component = reported("leg", Some("left"), Some("pose"));
    let other_role = reported("arm", Some("right"), Some("pose"));
    let other_aspect = reported("arm", Some("left"), Some("offset"));
    let bare = reported("arm", None, None);
    let reports = [
        &matched,
        &other_component,
        &other_role,
        &other_aspect,
        &bare,
    ];
    put_all(&store, &reports).await;
    let cases = [
        (
            LatestFilter {
                component: vec!["arm".to_string(), "wrist".to_string()],
                role: vec![Some("left".to_string())],
                aspect: vec![Some("pose".to_string())],
            },
            &matched,
        ),
        // `None` matches an entry without the slug
        (
            LatestFilter {
                role: vec![None],
                aspect: vec![None],
                ..LatestFilter::default()
            },
            &bare,
        ),
    ];

    for (filter, want) in cases {
        let latest = store.list(&filter, &HashSet::new()).await;
        let want = [listed(want, CalibrationUploadStatus::Dropped)];
        assert_eq!(latest.unwrap().entries, want, "{filter:?}");
    }
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
async fn content_is_not_found_for_an_unknown_report_or_a_missing_file() {
    let f = Fixture::new().await;
    let store = f.store();
    let arm = reported("arm", None, None);
    put_all(&store, &[&arm]).await;
    files::delete(&f.content_file(arm.report_id)).await.unwrap();

    for report_id in [Uuid::new_v4(), arm.report_id] {
        let err = store.content(report_id).await.unwrap_err();
        assert!(matches!(err, CalibrationsErr::NotFoundErr(_)), "{err:?}");
    }
}

// ================================ ERRORS ================================ //
#[tokio::test]
async fn a_cache_that_cannot_be_read_fails_the_call() {
    let f = Fixture::new().await;
    let store = f.store();
    let arm = reported("arm", None, None);
    put_all(&store, &[&arm]).await;
    f.block_content(arm.report_id).await;
    let unreadable_content = store.content(arm.report_id).await.unwrap_err();
    files::write_string(&f.layout.index(), "x", WriteOptions::OVERWRITE_ATOMIC)
        .await
        .unwrap();

    let errs = [
        unreadable_content,
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
    // a mark it can't read is logged, not raised
    store.mark_uploaded(arm.report_id).await;
}
