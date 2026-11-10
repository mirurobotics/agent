//! The latest calibrations store: its keys and capacity, write order,
//! listing, content, the prune when its worker starts, and shutdown.

// internal crates
use crate::test_utils::calibrations::{latest_cals_layout, latest_store, reported};
use crate::test_utils::filesys::dirs as test_dirs;
use miru_agent::calibrations::{latest, CalibrationsErr, Filter, LatestStore, ReportMetadata};
use miru_agent::disk::layout::LatestCalsLayout;
use miru_agent::filesys::{dirs, files, File, PathExt, WriteOptions};

// external crates
use futures::FutureExt;
use tokio::task::JoinHandle;
use uuid::Uuid;

/// The content every fixture report is put with.
const DATA: &str = "offset: 1\n";

struct Fixture {
    dir: test_dirs::TempDir,
    layout: LatestCalsLayout,
    store: LatestStore,
}

impl Fixture {
    async fn new() -> Self {
        Self::with_capacity(latest::DEFAULT_CAPACITY).await
    }

    /// A fixture whose store holds at most `capacity` keys.
    async fn with_capacity(capacity: usize) -> Self {
        let dir = test_dirs::temp("latest_cals_test").unwrap();
        Self {
            layout: latest_cals_layout(&dir),
            store: latest_store(&dir, capacity).await,
            dir,
        }
    }

    /// Spawn the store again over the same files. Its worker holds its own
    /// index in memory, apart from `store`'s.
    async fn reopen(&self) -> LatestStore {
        latest_store(&self.dir, latest::DEFAULT_CAPACITY).await
    }

    /// Spawn the store again over the same files, keeping its worker's handle.
    fn spawn(&self) -> (LatestStore, JoinHandle<()>) {
        LatestStore::spawn(64, latest_cals_layout(&self.dir), latest::DEFAULT_CAPACITY)
    }

    /// Every entry, in key order.
    async fn entries(&self) -> Vec<ReportMetadata> {
        self.store.list(&Filter::default()).await.unwrap()
    }

    /// The `report_id` each content file is named by, sorted.
    async fn content_ids(&self) -> Vec<Uuid> {
        let contents = dirs::files(&self.layout.contents()).await.unwrap();
        let mut ids: Vec<Uuid> = contents
            .iter()
            .map(|file| file.name().unwrap().parse().unwrap())
            .collect();
        ids.sort();
        ids
    }

    /// The content file for `report_id`.
    fn content_file(&self, report_id: Uuid) -> File {
        self.layout.contents().file(&report_id.to_string())
    }

    /// Write a content file for `report_id`, which no entry names.
    async fn write_orphan(&self, report_id: Uuid) {
        let file = self.content_file(report_id);
        files::write_string(&file, DATA, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();
    }

    /// Put a directory where `report_id`'s content file goes, so it can be
    /// neither read, written nor deleted.
    async fn block_content(&self, report_id: Uuid) {
        let file = self.content_file(report_id);
        files::delete(&file).await.unwrap();
        let blocker = self.layout.contents().subdir(file.name().unwrap());
        dirs::create(&blocker).await.unwrap();
    }

    /// Overwrite the index with what isn't JSON.
    async fn corrupt_index(&self) {
        files::write_string(&self.layout.index(), "{", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();
    }

    /// Put a directory where the index goes, so it can be neither read nor
    /// written, and return what the index held.
    async fn block_index(&self) -> String {
        let index = self.layout.index();
        let saved = files::read_string(&index).await.unwrap();
        files::delete(&index).await.unwrap();
        dirs::create(&self.layout.root.subdir("index.json"))
            .await
            .unwrap();
        saved
    }

    /// Undo [`Self::block_index`], writing back what the index held.
    async fn unblock_index(&self, saved: &str) {
        dirs::delete(&self.layout.root.subdir("index.json"))
            .await
            .unwrap();
        files::write_string(&self.layout.index(), saved, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();
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

/// Put each report with [`DATA`], in order.
async fn put_all(store: &LatestStore, reports: &[&ReportMetadata]) {
    for &metadata in reports {
        store.put(metadata.clone(), DATA).await.unwrap();
    }
}

// ================================ SPAWN ================================= //
#[tokio::test]
async fn spawn_deletes_the_contents_no_entry_names() {
    let f = Fixture::new().await;
    let arm = reported("arm", None, None);
    put_all(&f.store, &[&arm]).await;
    f.write_orphan(Uuid::new_v4()).await;

    f.reopen().await;

    assert_eq!(f.content_ids().await, [arm.report_id]);
}

// paused, so the read's retries don't wait in real time
#[tokio::test(start_paused = true)]
async fn spawn_keeps_the_contents_when_the_index_cannot_be_read() {
    let f = Fixture::new().await;
    let orphan = Uuid::new_v4();
    f.write_orphan(orphan).await;
    let index_dir = f.layout.root.subdir("index.json");
    dirs::create(&index_dir).await.unwrap();

    f.reopen().await;

    assert_eq!(f.content_ids().await, [orphan]);
}

#[tokio::test]
async fn spawn_keeps_the_contents_when_the_index_is_corrupt() {
    let f = Fixture::new().await;
    let orphan = Uuid::new_v4();
    f.write_orphan(orphan).await;
    f.corrupt_index().await;

    f.reopen().await;

    assert_eq!(f.content_ids().await, [orphan]);
}

// =============================== CAPACITY =============================== //
#[tokio::test]
async fn put_rejects_only_a_new_key_at_capacity() {
    let f = Fixture::with_capacity(2).await;
    let arm = reported("arm", None, None);
    let leg = reported("leg", None, None);
    // the second key fills the store
    put_all(&f.store, &[&arm, &leg]).await;

    let new_arm = newer(&arm);
    let existing_key = f.store.put(new_arm.clone(), DATA).await;
    let new_key = f.store.put(reported("hand", None, None), DATA).await;

    assert!(existing_key.is_ok(), "{existing_key:?}");
    let err = new_key.unwrap_err();
    assert!(matches!(err, CalibrationsErr::StoreFullErr(_)), "{err:?}");
    let mut content_ids = vec![new_arm.report_id, leg.report_id];
    content_ids.sort();
    assert_eq!(f.content_ids().await, content_ids);
    assert_eq!(f.entries().await, [new_arm, leg]);
}

// ================================= PUT ================================== //
#[tokio::test]
async fn put_adds_an_entry_and_its_content() {
    let f = Fixture::new().await;
    let arm = reported("arm", None, None);

    f.store.put(arm.clone(), DATA).await.unwrap();

    assert_eq!(f.entries().await, std::slice::from_ref(&arm));
    assert_eq!(f.content_ids().await, [arm.report_id]);
}

#[tokio::test]
async fn put_replaces_the_keys_entry_and_deletes_its_content() {
    let f = Fixture::new().await;
    let old = reported("arm", None, None);
    let new = newer(&old);
    put_all(&f.store, &[&old]).await;

    f.store.put(new.clone(), DATA).await.unwrap();

    assert_eq!(f.entries().await, std::slice::from_ref(&new));
    assert_eq!(f.content_ids().await, [new.report_id]);
}

#[tokio::test]
async fn putting_the_same_report_again_keeps_its_content() {
    let f = Fixture::new().await;
    let arm = reported("arm", None, None);

    put_all(&f.store, &[&arm, &arm]).await;

    assert_eq!(f.entries().await, std::slice::from_ref(&arm));
    assert_eq!(f.store.content(arm.report_id).await.unwrap(), DATA);
}

#[tokio::test]
async fn role_and_aspect_are_part_of_the_key() {
    let f = Fixture::new().await;
    let bare = reported("arm", None, None);
    let with_role = reported("arm", Some("left"), None);
    let with_aspect = reported("arm", None, Some("pose"));

    put_all(&f.store, &[&bare, &with_role, &with_aspect]).await;

    // in key order, a missing slug first
    assert_eq!(f.entries().await, [bare, with_aspect, with_role]);
}

#[tokio::test]
async fn put_fails_and_changes_nothing_when_the_content_cannot_be_written() {
    let f = Fixture::new().await;
    let arm = reported("arm", None, None);
    // a regular file where the contents directory should be
    let blocker = File::new(f.layout.contents().path().clone());
    files::write_string(&blocker, "x", WriteOptions::OVERWRITE_ATOMIC)
        .await
        .unwrap();

    let err = f.store.put(arm, DATA).await.unwrap_err();

    assert!(matches!(err, CalibrationsErr::FileSysErr(_)), "{err:?}");
    assert!(f.entries().await.is_empty());
}

#[tokio::test]
async fn put_fails_and_keeps_the_replaced_entry_when_the_content_cannot_be_written() {
    let f = Fixture::new().await;
    let old = reported("arm", None, None);
    let new = newer(&old);
    put_all(&f.store, &[&old]).await;
    f.block_content(new.report_id).await;

    let err = f.store.put(new, DATA).await.unwrap_err();

    assert!(matches!(err, CalibrationsErr::FileSysErr(_)), "{err:?}");
    assert_eq!(f.entries().await, std::slice::from_ref(&old));
    assert!(f.content_ids().await.contains(&old.report_id));
}

#[tokio::test]
async fn put_fails_and_keeps_the_entries_when_the_index_cannot_be_written() {
    let f = Fixture::new().await;
    let arm = reported("arm", None, None);
    put_all(&f.store, &[&arm]).await;
    f.block_index().await;
    let leg = reported("leg", None, None);

    let err = f.store.put(leg, DATA).await.unwrap_err();

    assert!(matches!(err, CalibrationsErr::FileSysErr(_)), "{err:?}");
    assert_eq!(f.entries().await, [arm]);
}

#[tokio::test]
async fn put_succeeds_when_the_replaced_content_cannot_be_deleted() {
    let f = Fixture::new().await;
    let old = reported("arm", None, None);
    let new = newer(&old);
    put_all(&f.store, &[&old]).await;
    f.block_content(old.report_id).await;

    f.store.put(new.clone(), DATA).await.unwrap();

    assert_eq!(f.entries().await, [new]);
}

// ================================= LIST ================================= //
#[tokio::test]
async fn list_sorts_by_component_then_role_then_aspect() {
    let f = Fixture::new().await;
    // in sorted order, where a missing role or aspect sorts first
    let sorted = [
        reported("arm", None, None),
        reported("arm", None, Some("pose")),
        reported("arm", Some("left"), None),
        reported("arm", Some("left"), Some("offset")),
        reported("arm", Some("left"), Some("pose")),
        reported("arm", Some("right"), None),
        reported("leg", None, None),
    ];
    let shuffled = [4, 0, 6, 2, 5, 1, 3].map(|i| &sorted[i]);
    put_all(&f.store, &shuffled).await;

    let entries = f.store.list(&Filter::default()).await.unwrap();

    assert_eq!(entries, sorted);
}

#[tokio::test]
async fn list_filters_by_component_role_and_aspect() {
    let f = Fixture::new().await;
    let matched = reported("arm", Some("left"), Some("pose"));
    // each misses the first filter on one slug
    let other_component = reported("leg", Some("left"), Some("pose"));
    let other_role = reported("arm", Some("right"), Some("pose"));
    let other_aspect = reported("arm", Some("left"), Some("offset"));
    let bare = reported("arm", None, None);
    // each lacks only one of role and aspect, so a `None` that matched anything
    // would list them
    let role_only = reported("arm", Some("left"), None);
    let aspect_only = reported("arm", None, Some("pose"));
    let reports = [
        &matched,
        &other_component,
        &other_role,
        &other_aspect,
        &bare,
        &role_only,
        &aspect_only,
    ];
    put_all(&f.store, &reports).await;
    let cases = [
        (
            Filter {
                component: vec!["arm".to_string(), "wrist".to_string()],
                role: vec![Some("left".to_string())],
                aspect: vec![Some("pose".to_string())],
            },
            &matched,
        ),
        // `None` matches an entry without the slug
        (
            Filter {
                role: vec![None],
                aspect: vec![None],
                ..Filter::default()
            },
            &bare,
        ),
    ];

    for (filter, want) in cases {
        let entries = f.store.list(&filter).await.unwrap();
        assert_eq!(entries, std::slice::from_ref(want), "{filter:?}");
    }
}

#[tokio::test]
async fn list_serves_the_entries_from_memory() {
    let f = Fixture::new().await;
    let arm = reported("arm", None, None);
    put_all(&f.store, &[&arm]).await;

    files::delete(&f.layout.index()).await.unwrap();

    assert_eq!(f.entries().await, [arm]);
}

// ================================ CONTENT =============================== //
#[tokio::test]
async fn content_returns_the_data() {
    let f = Fixture::new().await;
    let arm = reported("arm", None, None);
    put_all(&f.store, &[&arm]).await;

    let content = f.store.content(arm.report_id).await.unwrap();

    assert_eq!(content, DATA);
}

#[tokio::test]
async fn content_is_not_found_for_an_unknown_report() {
    let f = Fixture::new().await;
    put_all(&f.store, &[&reported("arm", None, None)]).await;

    let err = f.store.content(Uuid::new_v4()).await.unwrap_err();

    assert!(matches!(err, CalibrationsErr::NotFoundErr(_)), "{err:?}");
}

#[tokio::test]
async fn content_is_not_found_when_no_entry_names_it() {
    let f = Fixture::new().await;
    let unnamed = Uuid::new_v4();
    f.write_orphan(unnamed).await;

    let err = f.store.content(unnamed).await.unwrap_err();

    assert!(matches!(err, CalibrationsErr::NotFoundErr(_)), "{err:?}");
}

#[tokio::test]
async fn content_is_not_found_when_its_file_is_missing() {
    let f = Fixture::new().await;
    let arm = reported("arm", None, None);
    put_all(&f.store, &[&arm]).await;
    files::delete(&f.content_file(arm.report_id)).await.unwrap();

    let err = f.store.content(arm.report_id).await.unwrap_err();

    assert!(matches!(err, CalibrationsErr::NotFoundErr(_)), "{err:?}");
}

// ================================ ERRORS ================================ //
#[tokio::test]
async fn a_corrupt_index_at_spawn_fails_every_call() {
    let f = Fixture::new().await;
    let arm = reported("arm", None, None);
    put_all(&f.store, &[&arm]).await;
    f.corrupt_index().await;

    let store = f.reopen().await;

    let errs = [
        store.put(arm.clone(), DATA).await.unwrap_err(),
        store.list(&Filter::default()).await.unwrap_err(),
        store.content(arm.report_id).await.unwrap_err(),
    ];

    for err in errs {
        assert!(matches!(err, CalibrationsErr::FileSysErr(_)), "{err:?}");
    }
}

// The index is a directory until 10 ms, between the worker's first read and
// its first retry at 50 ms. Time is paused, so the order is fixed.
#[tokio::test(start_paused = true)]
async fn spawn_retries_an_index_read_that_fails_for_a_moment() {
    let f = Fixture::new().await;
    let arm = reported("arm", None, None);
    put_all(&f.store, &[&arm]).await;
    let saved = f.block_index().await;
    let (store, _) = f.spawn();
    let filter = Filter::default();
    let unblock = async {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        f.unblock_index(&saved).await;
    };

    let ((), entries) = tokio::join!(unblock, store.list(&filter));

    assert_eq!(entries.unwrap(), [arm]);
}

// paused, so the read's retries don't wait in real time
#[tokio::test(start_paused = true)]
async fn a_call_reads_an_index_that_could_not_be_read_at_spawn() {
    let f = Fixture::new().await;
    let arm = reported("arm", None, None);
    put_all(&f.store, &[&arm]).await;
    let saved = f.block_index().await;
    let store = f.reopen().await;

    f.unblock_index(&saved).await;

    let entries = store.list(&Filter::default()).await.unwrap();
    assert_eq!(entries, [arm]);
}

#[tokio::test]
async fn content_fails_when_its_file_cannot_be_read() {
    let f = Fixture::new().await;
    let arm = reported("arm", None, None);
    put_all(&f.store, &[&arm]).await;
    f.block_content(arm.report_id).await;

    let err = f.store.content(arm.report_id).await.unwrap_err();

    assert!(matches!(err, CalibrationsErr::FileSysErr(_)), "{err:?}");
}

// =============================== SHUTDOWN =============================== //
#[tokio::test]
async fn commands_after_shutdown_fail() {
    let f = Fixture::new().await;
    let (store, handle) = f.spawn();
    store.shutdown().await.unwrap();
    // joined, so the worker has dropped its receiver
    handle.await.unwrap();
    let arm = reported("arm", None, None);

    let errs = [
        store.put(arm.clone(), DATA).await.unwrap_err(),
        store.list(&Filter::default()).await.unwrap_err(),
        store.content(arm.report_id).await.unwrap_err(),
    ];

    for err in errs {
        assert!(
            matches!(err, CalibrationsErr::SendActorMessageErr(_)),
            "{err:?}"
        );
    }
}

#[tokio::test]
async fn a_command_queued_behind_shutdown_fails() {
    let f = Fixture::new().await;
    let (store, handle) = f.spawn();
    let filter = Filter::default();

    let (stopped, listed) = tokio::join!(store.shutdown(), store.list(&filter));

    stopped.unwrap();
    let err = listed.unwrap_err();
    assert!(
        matches!(err, CalibrationsErr::ReceiveActorMessageErr(_)),
        "{err:?}"
    );
    handle.await.unwrap();
}

#[tokio::test]
async fn a_caller_that_stops_waiting_does_not_stop_the_store() {
    let f = Fixture::new().await;
    let arm = reported("arm", None, None);
    let filter = Filter::default();

    // polled once: the command is sent, then its reply is abandoned
    assert!(f.store.list(&filter).now_or_never().is_none());

    put_all(&f.store, &[&arm]).await;
    assert_eq!(f.entries().await, [arm]);
}

#[tokio::test]
async fn dropping_every_handle_stops_the_store() {
    let f = Fixture::new().await;
    let (store, handle) = f.spawn();

    drop(store);

    handle.await.unwrap();
}
