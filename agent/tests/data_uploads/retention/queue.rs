// internal crates
use miru_agent::data_uploads::retention::{
    DeleteErr, DeleteQueueSnapshot, DeleteQueueSnapshotFile, Job, Queue, QueueEntry,
};
use miru_agent::filesys::{dirs, files, Dir, File, PathExt, WriteOptions};

// external crates
use chrono::{DateTime, Utc};
use uuid::Uuid;

const DEFAULT_CAPACITY: usize = 4096;

/// The instant every test sweeps at. Jobs made with `ttl_secs: 0` are due at
/// it; jobs with a large TTL are not.
fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(2000, 0).unwrap()
}

/// A deterministic `Job` — fixed size/digest/mtime derived from `name`, so
/// whole-struct equality holds across snapshot reloads. The queue never
/// touches the file's contents, so no file exists at the path.
fn make_job(name: &str, observed_secs: i64, ttl_secs: u64) -> Job {
    let observed_at = DateTime::from_timestamp(observed_secs, 0).unwrap();
    Job {
        file: File::new(format!("/data/{name}")),
        size: 42,
        digest: format!("sha256:{name}"),
        mtime: DateTime::from_timestamp(900, 0).unwrap(),
        first_observed_at: observed_at,
        last_observed_at: observed_at,
        ttl_secs,
        file_rule_id: "file_rule_1".to_string(),
        deployment_id: "dpl_1".to_string(),
    }
}

/// A fresh snapshot handle over `path`. Reopening the same path returns a
/// handle whose in-memory cache reflects what was previously persisted.
async fn open(path: &File) -> DeleteQueueSnapshotFile {
    DeleteQueueSnapshotFile::new_with_default(path.clone(), DeleteQueueSnapshot::default())
        .await
        .unwrap()
}

/// The paths of the due jobs, in selection order. `next_ready` leaves the
/// entry in place, so draining without removing would return the same entry
/// forever.
async fn drain(queue: &mut Queue) -> Vec<String> {
    let mut out = Vec::new();
    while let Some(entry) = queue.next_ready(now()) {
        out.push(entry.job.file.to_string());
        queue.remove(entry.id).await;
    }
    out
}

/// The paths persisted at `path`, in queue order. Non-destructive: it reads
/// the snapshot through a fresh handle rather than draining a queue, so it can
/// be called repeatedly within one test.
async fn on_disk(path: &File) -> Vec<String> {
    open(path)
        .await
        .read()
        .entries
        .iter()
        .map(|entry| entry.job.file.to_string())
        .collect()
}

mod from_snapshot {
    use super::*;

    #[tokio::test]
    async fn empty_snapshot_loads_empty_queue() {
        let dir = dirs::temp("delete-queue-test").unwrap();
        let path = dir.file("delete_queue.json");

        let queue = Queue::from_snapshot(8, open(&path).await);

        assert!(queue.is_empty());
    }

    /// Pins the persisted wire format: entries are `{id, job}`, with the job's
    /// fields nested rather than flattened.
    #[tokio::test]
    async fn raw_json_snapshot_loads() {
        let dir = dirs::temp("delete-queue-test").unwrap();
        let path = dir.file("delete_queue.json");
        let id = Uuid::new_v4();
        let raw = format!(
            concat!(
                r#"{{"entries":[{{"id":"{id}","job":{{"#,
                r#""file":"/data/a.log","size":42,"#,
                r#""digest":"sha256:a.log","mtime":"1970-01-01T00:15:00Z","#,
                r#""first_observed_at":"1970-01-01T00:16:40Z","#,
                r#""last_observed_at":"1970-01-01T00:16:40Z","ttl_secs":500,"#,
                r#""file_rule_id":"file_rule_1","deployment_id":"dpl_1"}}}}]}}"#,
            ),
            id = id,
        );
        files::write_string(&path, &raw, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let queue = Queue::from_snapshot(8, open(&path).await);

        let entry = queue.next_ready(now()).unwrap();
        assert_eq!(entry.id, id);
        assert_eq!(entry.job, make_job("a.log", 1000, 500));
    }

    /// An entry written without an `id` is still loadable, and the id it is
    /// given is a working handle for removal.
    #[tokio::test]
    async fn entry_without_id_gets_one() {
        let dir = dirs::temp("delete-queue-test").unwrap();
        let path = dir.file("delete_queue.json");
        let snapshot = DeleteQueueSnapshot {
            entries: vec![QueueEntry {
                id: Uuid::new_v4(),
                job: make_job("a.log", 1000, 0),
            }],
        };
        let mut value = serde_json::to_value(&snapshot).unwrap();
        for entry in value["entries"].as_array_mut().unwrap() {
            entry.as_object_mut().unwrap().remove("id");
        }
        files::write_string(&path, &value.to_string(), WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        // if deserialization failed, new_with_default would silently write an
        // empty default snapshot and next_ready below would find nothing.
        let mut queue = Queue::from_snapshot(8, open(&path).await);
        let entry = queue.next_ready(now()).unwrap();
        assert_eq!(entry.job.file.to_string(), "/data/a.log");
        assert!(queue.remove(entry.id).await.is_some());
        assert!(queue.is_empty());
    }
}

mod enqueue {
    use super::*;

    #[tokio::test]
    async fn appends_in_fifo_order() {
        let mut queue = Queue::new(DEFAULT_CAPACITY);
        for name in ["a.log", "b.log", "c.log"] {
            queue.enqueue(make_job(name, 1000, 0)).await.unwrap();
        }

        assert_eq!(
            drain(&mut queue).await,
            ["/data/a.log", "/data/b.log", "/data/c.log"]
        );
    }

    /// Identity is per entry, not per job: two *identical* jobs for one path
    /// are both queued, under distinct ids. No field of `Job` is a usable key.
    #[tokio::test]
    async fn duplicate_same_path_jobs_are_both_queued() {
        let mut queue = Queue::new(DEFAULT_CAPACITY);
        let job = make_job("a.log", 1000, 0);

        queue.enqueue(job.clone()).await.unwrap();
        queue.enqueue(job).await.unwrap();

        let first = queue.next_ready(now()).unwrap();
        queue.remove(first.id).await.unwrap();
        let second = queue.next_ready(now()).unwrap();
        assert_eq!(first.job, second.job);
        assert_ne!(first.id, second.id);
    }

    #[tokio::test]
    async fn persists() {
        let dir = dirs::temp("delete-queue-test").unwrap();
        let path = dir.file("delete_queue.json");

        {
            let mut queue = Queue::from_snapshot(DEFAULT_CAPACITY, open(&path).await);
            queue.enqueue(make_job("a.log", 1000, 500)).await.unwrap();
        }

        assert_eq!(on_disk(&path).await, ["/data/a.log"]);
    }

    #[tokio::test]
    async fn full_queue_returns_queue_full_err() {
        let mut queue = Queue::new(1);
        queue.enqueue(make_job("a.log", 1000, 0)).await.unwrap();

        // a new path is rejected at capacity and the queue is unchanged.
        let err = queue.enqueue(make_job("b.log", 1000, 0)).await.unwrap_err();
        let DeleteErr::QueueFullErr(full) = err else {
            panic!("expected QueueFullErr, got: {err:?}");
        };
        assert_eq!(full.capacity, 1);
        assert_eq!(full.file, "/data/b.log");

        // same-path is not a bypass: a duplicate also grows the queue, so it
        // is rejected too.
        let err = queue
            .enqueue(make_job("a.log", 1200, 30))
            .await
            .unwrap_err();
        assert!(matches!(err, DeleteErr::QueueFullErr(_)));
        assert_eq!(drain(&mut queue).await, ["/data/a.log"]);
    }

    #[tokio::test]
    async fn rejection_does_not_persist() {
        let dir = dirs::temp("delete-queue-test").unwrap();
        let path = dir.file("delete_queue.json");

        {
            let mut queue = Queue::from_snapshot(1, open(&path).await);
            queue.enqueue(make_job("a.log", 1000, 0)).await.unwrap();
            queue.enqueue(make_job("b.log", 1000, 0)).await.unwrap_err();
        }

        assert_eq!(on_disk(&path).await, ["/data/a.log"]);
    }

    #[tokio::test]
    async fn persist_failure_is_swallowed() {
        let dir = dirs::temp("delete-queue-test").unwrap();
        let path = dir.file("delete_queue.json");
        let mut queue = Queue::from_snapshot(DEFAULT_CAPACITY, open(&path).await);

        // make the snapshot path unwritable: a DIRECTORY now sits there.
        files::delete(&path).await.unwrap();
        dirs::create(&Dir::new(path.path().clone())).await.unwrap();

        // the enqueue reports success despite the underlying persist error.
        queue.enqueue(make_job("a.log", 1000, 0)).await.unwrap();
        assert_eq!(queue.len(), 1);
    }
}

mod next_ready {
    use super::*;

    #[tokio::test]
    async fn returns_due_entries_in_fifo_order() {
        let mut queue = Queue::new(DEFAULT_CAPACITY);
        queue.enqueue(make_job("a.log", 1000, 0)).await.unwrap();
        queue.enqueue(make_job("b.log", 1000, 500)).await.unwrap();

        // due_at is inclusive: b is due at exactly `now`.
        assert_eq!(drain(&mut queue).await, ["/data/a.log", "/data/b.log"]);
    }

    /// A not-due entry at the head does not hide a due entry behind it, and is
    /// not moved out of the way to expose it.
    #[tokio::test]
    async fn skips_not_due_entries() {
        let mut queue = Queue::new(DEFAULT_CAPACITY);
        queue
            .enqueue(make_job("waiting", 1000, 5000))
            .await
            .unwrap();
        queue.enqueue(make_job("due", 1000, 0)).await.unwrap();

        let entry = queue.next_ready(now()).unwrap();
        assert_eq!(entry.job.file.to_string(), "/data/due");
        queue.remove(entry.id).await.unwrap();

        assert!(queue.next_ready(now()).is_none());
        assert_eq!(queue.len(), 1);
    }

    #[tokio::test]
    async fn returns_none_when_nothing_is_due() {
        let mut queue = Queue::new(DEFAULT_CAPACITY);
        assert!(queue.next_ready(now()).is_none());

        queue.enqueue(make_job("a.log", 1000, 5000)).await.unwrap();
        assert!(queue.next_ready(now()).is_none());
    }

    #[tokio::test]
    async fn leaves_the_entry_on_disk_until_removed() {
        let dir = dirs::temp("delete-queue-test").unwrap();
        let path = dir.file("delete_queue.json");
        let mut queue = Queue::from_snapshot(DEFAULT_CAPACITY, open(&path).await);
        queue.enqueue(make_job("a.log", 1000, 0)).await.unwrap();
        queue.enqueue(make_job("b.log", 1000, 0)).await.unwrap();

        let entry = queue.next_ready(now()).unwrap();
        assert_eq!(on_disk(&path).await, ["/data/a.log", "/data/b.log"]);

        queue.remove(entry.id).await.unwrap();
        assert_eq!(on_disk(&path).await, ["/data/b.log"]);
    }
}

mod count_ready {
    use super::*;

    #[tokio::test]
    async fn counts_only_due_entries() {
        let mut queue = Queue::new(DEFAULT_CAPACITY);
        queue.enqueue(make_job("a.log", 1000, 0)).await.unwrap();
        queue
            .enqueue(make_job("waiting", 1000, 5000))
            .await
            .unwrap();
        queue.enqueue(make_job("b.log", 1000, 500)).await.unwrap();

        assert_eq!(queue.count_ready(now()), 2);
        assert_eq!(queue.len(), 3);
    }

    #[test]
    fn zero_when_empty() {
        assert_eq!(Queue::new(DEFAULT_CAPACITY).count_ready(now()), 0);
    }
}

mod remove {
    use super::*;

    #[tokio::test]
    async fn removes_the_entry_from_disk() {
        let dir = dirs::temp("delete-queue-test").unwrap();
        let path = dir.file("delete_queue.json");

        {
            let mut queue = Queue::from_snapshot(DEFAULT_CAPACITY, open(&path).await);
            queue.enqueue(make_job("a.log", 1000, 0)).await.unwrap();
            queue.enqueue(make_job("b.log", 1000, 0)).await.unwrap();
            let entry = queue.next_ready(now()).unwrap();
            let removed = queue.remove(entry.id).await.unwrap();
            assert_eq!(removed.id, entry.id);
        }

        assert_eq!(on_disk(&path).await, ["/data/b.log"]);
    }

    #[tokio::test]
    async fn unknown_id_is_ignored() {
        let mut queue = Queue::new(DEFAULT_CAPACITY);
        queue.enqueue(make_job("a.log", 1000, 0)).await.unwrap();

        assert!(queue.remove(Uuid::new_v4()).await.is_none());
        assert_eq!(queue.len(), 1);
    }

    /// Two identical jobs for one path are removable independently.
    #[tokio::test]
    async fn removing_one_duplicate_leaves_the_other() {
        let mut queue = Queue::new(DEFAULT_CAPACITY);
        let job = make_job("a.log", 1000, 0);
        queue.enqueue(job.clone()).await.unwrap();
        queue.enqueue(job).await.unwrap();

        let entry = queue.next_ready(now()).unwrap();
        queue.remove(entry.id).await.unwrap();

        assert_eq!(queue.len(), 1);
        assert_ne!(queue.next_ready(now()).unwrap().id, entry.id);
    }
}

mod requeue {
    use super::*;

    #[tokio::test]
    async fn moves_the_entry_to_the_tail() {
        let mut queue = Queue::new(DEFAULT_CAPACITY);
        queue.enqueue(make_job("a.log", 1000, 0)).await.unwrap();
        queue.enqueue(make_job("b.log", 1000, 0)).await.unwrap();

        // a retried entry goes behind the entries not yet visited, so a sweep
        // budgeted over the due entries sees each of them once.
        let entry = queue.next_ready(now()).unwrap();
        queue.requeue(entry).await;

        assert_eq!(queue.len(), 2);
        assert_eq!(drain(&mut queue).await, ["/data/b.log", "/data/a.log"]);
    }

    /// A requeue is a rotation of an entry the queue already holds, not an
    /// admission, so it is not capacity-gated and cannot grow the queue.
    #[tokio::test]
    async fn at_capacity_rotates_without_growing() {
        let mut queue = Queue::new(1);
        queue.enqueue(make_job("a.log", 1000, 0)).await.unwrap();

        let entry = queue.next_ready(now()).unwrap();
        queue.requeue(entry.clone()).await;

        assert_eq!(queue.len(), 1);
        assert_eq!(queue.next_ready(now()).unwrap().id, entry.id);
    }

    #[tokio::test]
    async fn survives_reload() {
        let dir = dirs::temp("delete-queue-test").unwrap();
        let path = dir.file("delete_queue.json");
        let id;

        {
            let mut queue = Queue::from_snapshot(DEFAULT_CAPACITY, open(&path).await);
            queue.enqueue(make_job("a.log", 1000, 0)).await.unwrap();
            queue.enqueue(make_job("b.log", 1000, 0)).await.unwrap();
            let entry = queue.next_ready(now()).unwrap();
            id = entry.id;
            queue.requeue(entry).await;
        }

        let reloaded = Queue::from_snapshot(DEFAULT_CAPACITY, open(&path).await);
        assert_eq!(on_disk(&path).await, ["/data/b.log", "/data/a.log"]);
        assert_ne!(reloaded.next_ready(now()).unwrap().id, id);
    }
}

/// The point of the refactor: a persist landing between an entry's selection
/// and its resolution must not write the selected entry out of the snapshot.
///
/// This is simulated at the `Queue` level because the actor cannot currently
/// interleave a command with a sweep — `Worker::run` awaits `sweep()` to
/// completion inside its match arm — so the interleaving is performed directly
/// on the queue. The test exists so that making the run loop responsive to
/// shutdown via `select!`, which is what wiring the Deleter to a sweep ticker
/// will require, cannot silently reintroduce the loss.
mod durability {
    use super::*;

    #[tokio::test]
    async fn persist_during_an_in_flight_entry_keeps_it_on_disk() {
        let dir = dirs::temp("delete-queue-test").unwrap();
        let path = dir.file("delete_queue.json");
        let mut queue = Queue::from_snapshot(DEFAULT_CAPACITY, open(&path).await);
        queue.enqueue(make_job("a.log", 1000, 0)).await.unwrap();
        queue.enqueue(make_job("b.log", 1000, 0)).await.unwrap();

        // the sweep selects `a` and holds it: this is the in-flight state.
        let in_flight = queue.next_ready(now()).unwrap();
        assert_eq!(in_flight.job.file.to_string(), "/data/a.log");

        // an Enqueue command serviced mid-sweep. Its persist is the write that
        // a `select!`-driven run loop would perform while `a` is in flight.
        queue.enqueue(make_job("c.log", 1000, 0)).await.unwrap();

        assert_eq!(
            on_disk(&path).await,
            ["/data/a.log", "/data/b.log", "/data/c.log"]
        );

        // `a` leaves disk only when the sweep resolves it.
        queue.remove(in_flight.id).await.unwrap();
        assert_eq!(on_disk(&path).await, ["/data/b.log", "/data/c.log"]);
    }
}
