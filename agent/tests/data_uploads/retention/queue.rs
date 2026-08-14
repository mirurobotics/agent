//! Delete-queue cases that are *not* shared with the upload queue.
//!
//! Everything the two queues have in common now lives in
//! `agent/tests/data_uploads/queue.rs`, which exercises the one generic
//! implementation against both production job types. What remains here is the
//! on-disk wire pin for `delete_queue.json` — a per-worker artifact — the TTL
//! readiness cases that only a retention job can express (`due_at` is a real
//! deadline rather than `MIN_UTC`), the sweep's `count_ready` budget, and the
//! mid-sweep-persist durability case the retention refactor was written for.

// internal crates
use miru_agent::data_uploads::retention::{
    DeleteQueueSnapshot, DeleteQueueSnapshotFile, Job, Queue,
};
use miru_agent::filesys::{dirs, files, File, WriteOptions};

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

        assert_eq!(queue.len(), 1);
        let entry = queue.next_ready(now()).unwrap();
        assert_eq!(entry.id, id);
        assert_eq!(entry.job, make_job("a.log", 1000, 500));
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
