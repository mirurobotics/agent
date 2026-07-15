// internal crates
use crate::data_uploads::queue::{enqueue, queue_suite, DEFAULT_CAPACITY};
use miru_agent::data_uploads::queue::QueueJob;
use miru_agent::data_uploads::retention::{
    DeleteQueueSnapshot, DeleteQueueSnapshotFile, Job, Queue,
};
use miru_agent::filesys::state_file::Options;
use miru_agent::filesys::{dirs, files, File, WriteOptions};

// external crates
use chrono::{DateTime, TimeDelta, Utc};
use uuid::Uuid;

/// The instant the pin selects at. The pinned job's TTL has elapsed by then.
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
    DeleteQueueSnapshotFile::open(
        path.clone(),
        Options {
            default: Some(DeleteQueueSnapshot::default()),
            ..Default::default()
        },
    )
    .await
    .unwrap()
}

/// A deterministic retention job whose TTL has yet to elapse at [`now`] unless
/// `ttl_secs` is small enough. Observed at 1000s, so a TTL of 1000s is due at
/// exactly [`now`].
fn retention_job_with_ttl(name: &str, ttl_secs: u64) -> Job {
    make_job(name, 1000, ttl_secs)
}

/// A retention job with a zero TTL, so it is due at [`now`].
fn retention_job(name: &str) -> Job {
    retention_job_with_ttl(name, 0)
}

queue_suite!(retention_job, "delete-generic-queue-test");

/// The other state of the `due_at` hook: a retention job is not ready until
/// its TTL has elapsed, and `next_ready` / `count_ready` both honour it.
#[tokio::test]
async fn a_job_whose_ttl_has_not_elapsed_is_not_ready() {
    let mut queue = Queue::new(DEFAULT_CAPACITY);
    enqueue(&mut queue, retention_job_with_ttl("waiting", 5000)).await;
    enqueue(&mut queue, retention_job_with_ttl("due", 500)).await;

    // due_at is inclusive: "due" is due at exactly `now`
    assert_eq!(queue.count_ready(now()), 1);
    assert_eq!(queue.next_ready(now()).unwrap().job.name(), "/data/due");

    // the waiting job becomes ready once its TTL elapses
    assert_eq!(queue.count_ready(now() + TimeDelta::hours(2)), 2);
}

mod wire {
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
