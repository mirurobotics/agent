//! The generic durable buffer, exercised with a minimal test job. The
//! calibration instantiation is covered in `agent/tests/calibrations/`.

// internal crates
use crate::test_utils::filesys::dirs as test_dirs;
use miru_agent::buffer::{Buffer, BufferErr, BufferJob, BufferOptions, Item};
use miru_agent::filesys::{dirs, files, Dir, File, PathExt, WriteOptions};
use miru_agent::queue::{QueueEntry, QueueJob};

// external crates
use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct TestJob {
    name: String,
    received_at: DateTime<Utc>,
}

impl QueueJob for TestJob {
    const LABEL: &'static str = "test";

    fn due_at(&self) -> DateTime<Utc> {
        DateTime::<Utc>::MIN_UTC
    }

    fn name(&self) -> String {
        self.name.clone()
    }
}

impl BufferJob for TestJob {
    fn received_at(&self) -> DateTime<Utc> {
        self.received_at
    }
}

type TestBuffer = Buffer<TestJob, Value>;
type TestItem = Item<TestJob, Value>;

const OPTIONS: BufferOptions = BufferOptions {
    max_items: 1000,
    max_bytes: 256 * 1024 * 1024,
};

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(5000, 0).unwrap()
}

/// A job told apart from others by its `name`.
fn job(name: &str, received_at_secs: i64) -> TestJob {
    TestJob {
        name: name.to_string(),
        received_at: DateTime::from_timestamp(received_at_secs, 0).unwrap(),
    }
}

fn item(name: &str) -> TestItem {
    Item {
        job: job(name, 1000),
        content: json!({ "format": "json", "data": { "name": name, "offset": [1, 2, 3] } }),
    }
}

/// An item whose file is exactly `bytes` long once serialized.
fn item_of_size(name: &str, bytes: u64) -> TestItem {
    let mut item = item(name);
    item.content = json!("");
    let base = serialized_len(&item);
    item.content = json!("x".repeat((bytes - base) as usize));
    assert_eq!(serialized_len(&item), bytes);
    item
}

fn serialized_len(item: &TestItem) -> u64 {
    serde_json::to_vec(item).unwrap().len() as u64
}

struct Fixture {
    _dir: test_dirs::TempDir,
    root: Dir,
}

impl Fixture {
    fn new() -> Self {
        let dir = test_dirs::temp("buffer_test").unwrap();
        let root = dir.to_dir().subdir("buffer");
        Self { _dir: dir, root }
    }

    fn content(&self) -> Dir {
        self.root.subdir("content")
    }

    fn queue_file(&self) -> File {
        self.root.file("queue.json")
    }

    async fn try_open(&self) -> Result<TestBuffer, BufferErr> {
        TestBuffer::open(self.queue_file(), self.content(), OPTIONS).await
    }

    async fn open(&self) -> TestBuffer {
        self.open_with(OPTIONS).await
    }

    async fn open_with(&self, options: BufferOptions) -> TestBuffer {
        TestBuffer::open(self.queue_file(), self.content(), options)
            .await
            .unwrap()
    }

    fn item_file(&self, id: Uuid) -> File {
        self.content().file(&format!("{id}.json"))
    }
}

/// The buffered items' names, oldest first.
fn names(buffer: &TestBuffer) -> Vec<String> {
    buffer.jobs().into_iter().map(|job| job.name).collect()
}

fn strings(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| name.to_string()).collect()
}

/// Take and remove every ready item, oldest first.
async fn drain(buffer: &mut TestBuffer) -> Vec<String> {
    let mut out = Vec::new();
    while let Some(entry) = buffer.next_ready(now()) {
        out.push(entry.job.name.clone());
        buffer.remove(entry.id).await.unwrap();
    }
    out
}

async fn enqueue_all(buffer: &mut TestBuffer, names: &[&str]) -> Vec<Uuid> {
    let mut ids = Vec::new();
    for name in names {
        ids.push(buffer.enqueue(item(name)).await.unwrap());
    }
    ids
}

/// The names `queue.json` holds, read straight from disk.
async fn queue_on_disk(fixture: &Fixture) -> Vec<String> {
    let value: Value = files::read_json(&fixture.queue_file()).await.unwrap();
    value["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["job"]["name"].as_str().unwrap().to_string())
        .collect()
}

pub mod open {
    use super::*;

    #[tokio::test]
    async fn creates_an_empty_buffer() {
        let fixture = Fixture::new();
        let buffer = fixture.open().await;

        assert!(buffer.is_empty());
        assert_eq!(buffer.len(), 0);
        assert_eq!(buffer.used_bytes(), 0);
        assert!(fixture.queue_file().exists());
        assert!(fixture.content().exists());
        assert!(queue_on_disk(&fixture).await.is_empty());
    }

    #[tokio::test]
    async fn survives_a_restart() {
        let fixture = Fixture::new();
        let (ids, used) = {
            let mut buffer = fixture.open().await;
            let ids = enqueue_all(&mut buffer, &["a", "b", "c"]).await;
            (ids, buffer.used_bytes())
        };

        let mut buffer = fixture.open().await;

        assert_eq!(names(&buffer), strings(&["a", "b", "c"]));
        assert_eq!(buffer.used_bytes(), used);
        assert_eq!(buffer.next_ready(now()).unwrap().id, ids[0]);
        let file = buffer.read(ids[1]).await.unwrap();
        assert_eq!(file.job, item("b").job);
        assert_eq!(file.content, item("b").content);
        assert_eq!(drain(&mut buffer).await, strings(&["a", "b", "c"]));
    }

    #[tokio::test]
    async fn removes_item_files_with_no_queue_entry() {
        let fixture = Fixture::new();
        let ids = {
            let mut buffer = fixture.open().await;
            enqueue_all(&mut buffer, &["a"]).await
        };
        // a crash between the item file and the queue entry leaves this
        let orphan = fixture.item_file(Uuid::new_v4());
        files::write_string(&orphan, "{}", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let buffer = fixture.open().await;

        assert!(!orphan.exists());
        assert!(fixture.item_file(ids[0]).exists());
        assert_eq!(names(&buffer), strings(&["a"]));
    }

    #[tokio::test]
    async fn leaves_files_that_are_not_item_files() {
        let fixture = Fixture::new();
        let content = fixture.content();
        let id = Uuid::new_v4();
        let others = [
            content.file(&format!("{id}.json.corrupt")),
            content.file(&format!(".{id}.json.tmp")),
            content.file(&format!("{}.json", id.simple())),
            content.file(&format!("{}.json", id.to_string().to_uppercase())),
            content.file("notes.txt"),
            content.file("not-a-uuid.json"),
        ];
        for file in &others {
            files::write_string(file, "x", WriteOptions::OVERWRITE_ATOMIC)
                .await
                .unwrap();
        }

        let buffer = fixture.open().await;

        assert!(buffer.is_empty());
        for file in &others {
            assert!(file.exists(), "{file} was removed");
        }
    }

    #[tokio::test]
    async fn drops_queue_entries_whose_item_file_is_missing() {
        let fixture = Fixture::new();
        let ids = {
            let mut buffer = fixture.open().await;
            enqueue_all(&mut buffer, &["a", "b", "c"]).await
        };
        files::delete(&fixture.item_file(ids[1])).await.unwrap();

        let buffer = fixture.open().await;

        assert_eq!(names(&buffer), strings(&["a", "c"]));
        assert_eq!(queue_on_disk(&fixture).await, strings(&["a", "c"]));
        assert_eq!(
            buffer.used_bytes(),
            serialized_len(&item("a")) + serialized_len(&item("c"))
        );
    }

    #[tokio::test]
    async fn rebuilds_an_unreadable_queue_from_the_item_files() {
        let fixture = Fixture::new();
        let mut ids = Vec::new();
        {
            let mut buffer = fixture.open().await;
            // enqueued out of received_at order; the rebuild orders by
            // received_at since the lost queue order cannot be recovered
            for (name, secs) in [("late", 3000), ("early", 1000), ("middle", 2000)] {
                let item = Item {
                    job: job(name, secs),
                    ..item(name)
                };
                ids.push(buffer.enqueue(item).await.unwrap());
            }
        }
        files::write_string(
            &fixture.queue_file(),
            "{not json",
            WriteOptions::OVERWRITE_ATOMIC,
        )
        .await
        .unwrap();
        // an unreadable item file is set aside, not uploaded or deleted
        let broken = Uuid::new_v4();
        files::write_string(
            &fixture.item_file(broken),
            "{not json",
            WriteOptions::OVERWRITE_ATOMIC,
        )
        .await
        .unwrap();

        let mut buffer = fixture.open().await;

        assert_eq!(names(&buffer), strings(&["early", "middle", "late"]));
        assert_eq!(
            queue_on_disk(&fixture).await,
            strings(&["early", "middle", "late"])
        );
        // each rebuilt entry keeps the id that names its item file
        let entry = buffer.next_ready(now()).unwrap();
        assert_eq!(entry.id, ids[1]);
        let aside = fixture.root.file("queue.json.corrupt");
        assert_eq!(files::read_string(&aside).await.unwrap(), "{not json");
        let content = fixture.content();
        assert!(content.file(&format!("{broken}.json.corrupt")).exists());
        assert!(!fixture.item_file(broken).exists());
        assert_eq!(
            drain(&mut buffer).await,
            strings(&["early", "middle", "late"])
        );
    }

    /// Lock the content directory so it cannot be listed, run `f`, then
    /// restore it so the temp dir can be cleaned up.
    #[cfg(unix)]
    async fn with_unlistable_content<T>(
        fixture: &Fixture,
        f: impl std::future::Future<Output = T>,
    ) -> T {
        use std::os::unix::fs::PermissionsExt;
        let content = fixture.content();
        let mode = |bits| std::fs::Permissions::from_mode(bits);
        dirs::set_permissions(&content, mode(0o300)).await.unwrap();
        let out = f.await;
        dirs::set_permissions(&content, mode(0o700)).await.unwrap();
        out
    }

    /// A transient listing failure must not read as an empty directory: that
    /// would drop every queue entry, and the next open would delete every
    /// item file as an orphan.
    #[cfg(unix)]
    #[tokio::test]
    async fn listing_failure_fails_the_open_and_loses_nothing() {
        let fixture = Fixture::new();
        let ids = {
            let mut buffer = fixture.open().await;
            enqueue_all(&mut buffer, &["a", "b"]).await
        };

        let result = with_unlistable_content(&fixture, fixture.try_open()).await;

        assert!(matches!(result, Err(BufferErr::FileSysErr(_))));
        assert_eq!(queue_on_disk(&fixture).await, strings(&["a", "b"]));
        let buffer = fixture.open().await;
        assert_eq!(names(&buffer), strings(&["a", "b"]));
        assert!(fixture.item_file(ids[0]).exists());
        assert!(fixture.item_file(ids[1]).exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn listing_failure_during_a_rebuild_keeps_the_snapshot_for_the_next_open() {
        let fixture = Fixture::new();
        {
            let mut buffer = fixture.open().await;
            enqueue_all(&mut buffer, &["a", "b"]).await;
        }
        files::write_string(
            &fixture.queue_file(),
            "{not json",
            WriteOptions::OVERWRITE_ATOMIC,
        )
        .await
        .unwrap();

        let result = with_unlistable_content(&fixture, fixture.try_open()).await;

        assert!(matches!(result, Err(BufferErr::FileSysErr(_))));
        // the unreadable snapshot stays put, so the next open rebuilds again
        // rather than starting from an empty queue
        let raw = files::read_string(&fixture.queue_file()).await.unwrap();
        assert_eq!(raw, "{not json");
        let buffer = fixture.open().await;
        // same received_at, so the rebuild's order between them is by id
        let mut rebuilt = names(&buffer);
        rebuilt.sort();
        assert_eq!(rebuilt, strings(&["a", "b"]));
    }

    /// A read error other than a parse failure may be transient, so the
    /// snapshot, with its attempt counts and order, stays for the next open.
    #[tokio::test]
    async fn snapshot_read_failure_fails_the_open_and_keeps_the_snapshot() {
        let fixture = Fixture::new();
        let ids = {
            let mut buffer = fixture.open().await;
            enqueue_all(&mut buffer, &["a"]).await
        };
        let saved = files::read_string(&fixture.queue_file()).await.unwrap();
        files::delete(&fixture.queue_file()).await.unwrap();
        // a directory in its place reads with an I/O error, not a parse error
        let blocker = Dir::new(fixture.queue_file().path().clone());
        dirs::create(&blocker).await.unwrap();

        let result = fixture.try_open().await;

        assert!(matches!(result, Err(BufferErr::FileSysErr(_))));
        assert!(!fixture.root.file("queue.json.corrupt").exists());
        assert!(fixture.item_file(ids[0]).exists());
        dirs::delete(&blocker).await.unwrap();
        files::write_string(
            &fixture.queue_file(),
            &saved,
            WriteOptions::OVERWRITE_ATOMIC,
        )
        .await
        .unwrap();
        let buffer = fixture.open().await;
        assert_eq!(names(&buffer), strings(&["a"]));
    }

    /// A item file that cannot be read (as opposed to parsed) during a
    /// rebuild fails the open instead of being set aside.
    #[cfg(unix)]
    #[tokio::test]
    async fn report_read_failure_during_a_rebuild_fails_the_open() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = Fixture::new();
        let ids = {
            let mut buffer = fixture.open().await;
            enqueue_all(&mut buffer, &["a"]).await
        };
        files::delete(&fixture.queue_file()).await.unwrap();
        let item = fixture.item_file(ids[0]);
        let mode = |bits| std::fs::Permissions::from_mode(bits);
        std::fs::set_permissions(item.path(), mode(0o000)).unwrap();

        let result = fixture.try_open().await;

        std::fs::set_permissions(item.path(), mode(0o600)).unwrap();
        assert!(matches!(result, Err(BufferErr::FileSysErr(_))));
        assert!(item.exists());
        let buffer = fixture.open().await;
        assert_eq!(names(&buffer), strings(&["a"]));
    }

    #[tokio::test]
    async fn rebuilds_a_missing_queue_from_the_item_files() {
        let fixture = Fixture::new();
        let ids = {
            let mut buffer = fixture.open().await;
            enqueue_all(&mut buffer, &["a"]).await
        };
        files::delete(&fixture.queue_file()).await.unwrap();

        let buffer = fixture.open().await;

        assert_eq!(names(&buffer), strings(&["a"]));
        assert_eq!(buffer.next_ready(now()).unwrap().id, ids[0]);
        assert!(fixture.item_file(ids[0]).exists());
        assert_eq!(queue_on_disk(&fixture).await, strings(&["a"]));
    }

    #[tokio::test]
    async fn loads_a_backlog_over_a_lowered_cap() {
        let fixture = Fixture::new();
        {
            let mut buffer = fixture.open().await;
            enqueue_all(&mut buffer, &["a", "b", "c"]).await;
        }
        let options = BufferOptions {
            max_items: 2,
            ..OPTIONS
        };

        let mut buffer = fixture.open_with(options).await;

        assert_eq!(buffer.len(), 3);
        let err = buffer.enqueue(item("d")).await.unwrap_err();
        assert!(matches!(err, BufferErr::BufferFullErr(_)), "{err:?}");
    }

    #[tokio::test]
    async fn fails_when_the_buffer_root_is_a_file() {
        let fixture = Fixture::new();
        files::write_string(
            &File::new(fixture.root.path().clone()),
            "x",
            WriteOptions::OVERWRITE_ATOMIC,
        )
        .await
        .unwrap();

        let result = fixture.try_open().await;

        assert!(matches!(result, Err(BufferErr::FileSysErr(_))));
    }
}

pub mod enqueue {
    use super::*;

    /// Every item in a single listing of the content directory.
    async fn content_files(fixture: &Fixture) -> Vec<File> {
        dirs::files(&fixture.content()).await.unwrap()
    }

    #[tokio::test]
    async fn writes_the_item_file_and_the_queue_entry() {
        let fixture = Fixture::new();
        let mut buffer = fixture.open().await;

        let id = buffer.enqueue(item("a")).await.unwrap();

        let raw = files::read_string(&fixture.item_file(id)).await.unwrap();
        let value: Value = serde_json::from_str(&raw).unwrap();
        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["content", "job"], "{raw}");
        assert_eq!(value["job"]["name"], "a", "{raw}");
        assert_eq!(value["content"], item("a").content, "{raw}");
        assert_eq!(queue_on_disk(&fixture).await, strings(&["a"]));
        let entries: Value = files::read_json(&fixture.queue_file()).await.unwrap();
        assert_eq!(entries["entries"][0]["id"], id.to_string());
        assert_eq!(buffer.used_bytes(), raw.len() as u64);
    }

    #[tokio::test]
    async fn keeps_fifo_order() {
        let fixture = Fixture::new();
        let mut buffer = fixture.open().await;

        enqueue_all(&mut buffer, &["c", "a", "b"]).await;

        assert_eq!(drain(&mut buffer).await, strings(&["c", "a", "b"]));
        assert!(buffer.is_empty());
    }

    #[tokio::test]
    async fn identical_reports_are_both_buffered() {
        let fixture = Fixture::new();
        let mut buffer = fixture.open().await;

        let ids = enqueue_all(&mut buffer, &["a", "a"]).await;

        assert_ne!(ids[0], ids[1]);
        assert_eq!(drain(&mut buffer).await, strings(&["a", "a"]));
    }

    #[tokio::test]
    async fn rejects_at_the_report_cap() {
        let fixture = Fixture::new();
        let options = BufferOptions {
            max_items: 2,
            ..OPTIONS
        };
        let mut buffer = fixture.open_with(options).await;
        enqueue_all(&mut buffer, &["a", "b"]).await;

        let err = buffer.enqueue(item("c")).await.unwrap_err();

        let BufferErr::BufferFullErr(err) = err else {
            panic!("expected BufferFullErr, got {err:?}");
        };
        assert_eq!((err.items, err.max_items), (2, 2));
        assert_eq!(content_files(&fixture).await.len(), 2);
        assert_eq!(names(&buffer), strings(&["a", "b"]));
        assert_eq!(queue_on_disk(&fixture).await, strings(&["a", "b"]));
    }

    #[tokio::test]
    async fn rejects_at_the_byte_cap() {
        let fixture = Fixture::new();
        let options = BufferOptions {
            max_items: 100,
            max_bytes: 1000,
        };
        let mut buffer = fixture.open_with(options).await;
        buffer.enqueue(item_of_size("a", 600)).await.unwrap();
        // exactly at the cap is accepted
        buffer.enqueue(item_of_size("b", 400)).await.unwrap();
        assert_eq!(buffer.used_bytes(), 1000);

        let err = buffer.enqueue(item_of_size("c", 300)).await.unwrap_err();

        let BufferErr::BufferFullErr(err) = err else {
            panic!("expected BufferFullErr, got {err:?}");
        };
        assert_eq!(
            (err.bytes, err.max_bytes, err.item_bytes),
            (1000, 1000, 300)
        );
        assert_eq!(content_files(&fixture).await.len(), 2);
        assert_eq!(names(&buffer), strings(&["a", "b"]));
    }

    #[tokio::test]
    async fn accepts_again_once_a_report_is_removed() {
        let fixture = Fixture::new();
        let options = BufferOptions {
            max_items: 100,
            max_bytes: 1000,
        };
        let mut buffer = fixture.open_with(options).await;
        buffer.enqueue(item_of_size("a", 600)).await.unwrap();
        assert!(buffer.enqueue(item_of_size("b", 600)).await.is_err());

        let entry = buffer.next_ready(now()).unwrap();
        buffer.remove(entry.id).await.unwrap();

        buffer.enqueue(item_of_size("b", 600)).await.unwrap();
        assert_eq!(buffer.used_bytes(), 600);
    }

    #[tokio::test]
    async fn queue_write_failure_rejects_and_removes_the_item_file() {
        let fixture = Fixture::new();
        let mut buffer = fixture.open().await;
        buffer.enqueue(item("a")).await.unwrap();
        let used = buffer.used_bytes();

        // make queue.json unwritable: a DIRECTORY now sits there
        files::delete(&fixture.queue_file()).await.unwrap();
        dirs::create(&Dir::new(fixture.queue_file().path().clone()))
            .await
            .unwrap();

        let err = buffer.enqueue(item("b")).await.unwrap_err();

        assert!(matches!(err, BufferErr::EnqueueErr(_)), "{err:?}");
        assert_eq!(content_files(&fixture).await.len(), 1);
        assert_eq!(names(&buffer), strings(&["a"]));
        assert_eq!(buffer.used_bytes(), used);
    }

    #[tokio::test]
    async fn item_file_write_failure_rejects() {
        let fixture = Fixture::new();
        let mut buffer = fixture.open().await;
        // a file where the content directory should be
        let content = fixture.content();
        dirs::delete(&content).await.unwrap();
        files::write_string(
            &File::new(content.path().clone()),
            "x",
            WriteOptions::OVERWRITE_ATOMIC,
        )
        .await
        .unwrap();

        let err = buffer.enqueue(item("a")).await.unwrap_err();

        assert!(matches!(err, BufferErr::FileSysErr(_)), "{err:?}");
        assert!(buffer.is_empty());
        assert!(queue_on_disk(&fixture).await.is_empty());
    }
}

pub mod remove {
    use super::*;

    #[tokio::test]
    async fn deletes_the_entry_and_its_item_file() {
        let fixture = Fixture::new();
        let mut buffer = fixture.open().await;
        let ids = enqueue_all(&mut buffer, &["a", "b"]).await;

        let removed = buffer.remove(ids[0]).await.unwrap();

        assert_eq!(removed.job.name, "a");
        assert!(!fixture.item_file(ids[0]).exists());
        assert!(fixture.item_file(ids[1]).exists());
        assert_eq!(queue_on_disk(&fixture).await, strings(&["b"]));
        assert_eq!(buffer.used_bytes(), serialized_len(&item("b")));
    }

    #[tokio::test]
    async fn unknown_id_is_ignored() {
        let fixture = Fixture::new();
        let mut buffer = fixture.open().await;
        enqueue_all(&mut buffer, &["a"]).await;

        assert!(buffer.remove(Uuid::new_v4()).await.is_none());
        assert_eq!(buffer.len(), 1);
    }
}

pub mod park {
    use super::*;

    fn failed(fixture: &Fixture) -> Dir {
        fixture.root.subdir("failed")
    }

    #[tokio::test]
    async fn moves_the_item_file_and_drops_the_entry() {
        let fixture = Fixture::new();
        let mut buffer = fixture.open().await;
        let ids = enqueue_all(&mut buffer, &["a", "b"]).await;

        let parked = buffer.park(ids[0], &failed(&fixture)).await.unwrap();

        let parked = parked.unwrap();
        assert_eq!((parked.id, parked.job.name), (ids[0], "a".to_string()));
        assert_eq!(names(&buffer), strings(&["b"]));
        assert_eq!(queue_on_disk(&fixture).await, strings(&["b"]));
        assert_eq!(buffer.used_bytes(), serialized_len(&item("b")));
        let moved = failed(&fixture).file(&format!("{}.json", ids[0]));
        let moved: TestItem = files::read_json(&moved).await.unwrap();
        assert_eq!(moved, item("a"));
        assert!(!fixture.item_file(ids[0]).exists());
    }

    #[tokio::test]
    async fn parked_items_stay_out_after_reopen() {
        let fixture = Fixture::new();
        let ids = {
            let mut buffer = fixture.open().await;
            let ids = enqueue_all(&mut buffer, &["a", "b"]).await;
            buffer.park(ids[0], &failed(&fixture)).await.unwrap();
            ids
        };

        let buffer = fixture.open().await;

        assert_eq!(names(&buffer), strings(&["b"]));
        assert!(failed(&fixture).file(&format!("{}.json", ids[0])).exists());
    }

    #[tokio::test]
    async fn unknown_id_is_none() {
        let fixture = Fixture::new();
        let mut buffer = fixture.open().await;
        enqueue_all(&mut buffer, &["a"]).await;

        let parked = buffer.park(Uuid::new_v4(), &failed(&fixture)).await;

        assert!(parked.unwrap().is_none());
        assert_eq!(buffer.len(), 1);
        assert!(!failed(&fixture).exists());
    }

    #[tokio::test]
    async fn failure_keeps_the_entry() {
        let fixture = Fixture::new();
        let mut buffer = fixture.open().await;
        let ids = enqueue_all(&mut buffer, &["a"]).await;
        let used = buffer.used_bytes();
        // a regular file where the destination directory should be
        files::write_string(
            &File::new(failed(&fixture).path().clone()),
            "x",
            WriteOptions::OVERWRITE_ATOMIC,
        )
        .await
        .unwrap();

        let err = buffer.park(ids[0], &failed(&fixture)).await.unwrap_err();

        assert!(matches!(err, BufferErr::FileSysErr(_)), "{err:?}");
        assert_eq!(buffer.len(), 1);
        assert_eq!(buffer.used_bytes(), used);
        assert_eq!(queue_on_disk(&fixture).await, strings(&["a"]));
        assert!(fixture.item_file(ids[0]).exists());
    }

    #[tokio::test]
    async fn move_failure_keeps_the_entry() {
        let fixture = Fixture::new();
        let mut buffer = fixture.open().await;
        let ids = enqueue_all(&mut buffer, &["a"]).await;
        // a directory where the parked file should land
        let dest = failed(&fixture).subdir(format!("{}.json", ids[0]));
        dirs::create(&dest).await.unwrap();

        let err = buffer.park(ids[0], &failed(&fixture)).await.unwrap_err();

        assert!(matches!(err, BufferErr::FileSysErr(_)), "{err:?}");
        assert_eq!(names(&buffer), strings(&["a"]));
        assert_eq!(queue_on_disk(&fixture).await, strings(&["a"]));
        assert!(fixture.item_file(ids[0]).exists());
    }
}

pub mod requeue {
    use super::*;

    #[tokio::test]
    async fn backoff_survives_a_restart() {
        let fixture = Fixture::new();
        let deadline = now() + TimeDelta::minutes(5);
        let ids = {
            let mut buffer = fixture.open().await;
            let ids = enqueue_all(&mut buffer, &["a", "b"]).await;
            let entry = buffer.next_ready(now()).unwrap();
            buffer
                .requeue(QueueEntry {
                    attempts: 3,
                    next_attempt_at: Some(deadline),
                    ..entry
                })
                .await;
            assert_eq!(
                buffer.earliest_next_attempt(),
                Some(DateTime::<Utc>::MIN_UTC)
            );
            ids
        };

        let mut buffer = fixture.open().await;

        assert_eq!(names(&buffer), strings(&["b", "a"]));
        let ready = buffer.next_ready(now()).unwrap();
        assert_eq!(ready.id, ids[1]);
        buffer.remove(ready.id).await.unwrap();
        // `a` waits out its backoff, then comes back with its attempt count
        assert!(buffer.next_ready(now()).is_none());
        assert_eq!(buffer.earliest_next_attempt(), Some(deadline));
        let waiting = buffer.next_ready(deadline).unwrap();
        assert_eq!(waiting.id, ids[0]);
        assert_eq!(waiting.attempts, 3);
        assert!(buffer.read(ids[0]).await.is_ok());
    }

    #[tokio::test]
    async fn an_entry_that_was_removed_is_ignored() {
        let fixture = Fixture::new();
        let mut buffer = fixture.open().await;
        let ids = enqueue_all(&mut buffer, &["a", "b"]).await;
        let stale = buffer.next_ready(now()).unwrap();
        buffer.remove(ids[0]).await.unwrap();

        buffer.requeue(stale).await;

        assert_eq!(names(&buffer), strings(&["b"]));
        assert_eq!(queue_on_disk(&fixture).await, strings(&["b"]));
        assert!(!fixture.item_file(ids[0]).exists());
    }
}

pub mod read {
    use super::*;

    #[tokio::test]
    async fn returns_the_job_and_content() {
        let fixture = Fixture::new();
        let mut buffer = fixture.open().await;
        let id = buffer.enqueue(item("a")).await.unwrap();

        let file = buffer.read(id).await.unwrap();

        assert_eq!(file.job, item("a").job);
        assert_eq!(file.content, item("a").content);
    }

    #[tokio::test]
    async fn errors_for_an_unknown_entry() {
        let fixture = Fixture::new();
        let buffer = fixture.open().await;

        let err = buffer.read(Uuid::new_v4()).await.unwrap_err();

        assert!(matches!(err, BufferErr::FileSysErr(_)), "{err:?}");
    }
}
