// internal crates
use crate::queue::file::{enqueue, queue_suite, DEFAULT_CAPACITY};
use crate::test_utils::filesys::{abs_path, dirs as test_dirs};
use miru_agent::data_uploads::retention::{
    DeleteQueueSnapshot, DeleteQueueSnapshotFile, Expiry, Job, Queue,
};
use miru_agent::filesys::{files, File, WriteOptions};
use miru_agent::queue::Persistence;

// external crates
use chrono::{DateTime, Utc};
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
        expiry: Default::default(),
    }
}

/// A fresh snapshot handle over `path`. Reopening the same path returns a
/// handle whose in-memory cache reflects what was previously persisted.
async fn open(path: &File) -> DeleteQueueSnapshotFile {
    DeleteQueueSnapshotFile::new_with_default(path.clone(), DeleteQueueSnapshot::default())
        .await
        .unwrap()
}

/// A deterministic retention job observed at 1000s with `ttl_secs`.
fn retention_job_with_ttl(name: &str, ttl_secs: u64) -> Job {
    make_job(name, 1000, ttl_secs)
}

/// A retention job with a zero TTL.
fn retention_job(name: &str) -> Job {
    retention_job_with_ttl(name, 0)
}

queue_suite!(retention_job, "delete-generic-queue-test");

/// The queue never holds a retention job back on its TTL: the deleter decides
/// when the TTL has run out (see `DeleterImpl::sweep`), because that takes
/// server and monotonic time, not the system clock the queue is given.
#[tokio::test]
async fn a_job_is_ready_whatever_its_ttl() {
    let mut queue = Queue::new(DEFAULT_CAPACITY);
    enqueue(&mut queue, retention_job_with_ttl("waiting", 5000)).await;

    assert_eq!(queue.count_ready(DateTime::<Utc>::MIN_UTC), 1);
}

mod wire {
    use super::*;

    /// Pins the persisted wire format: entries are `{id, job}`, with the job's
    /// fields nested rather than flattened.
    #[tokio::test]
    async fn raw_json_snapshot_loads() {
        let dir = test_dirs::temp("delete-queue-test").unwrap();
        let path = dir.file("delete_queue.json");
        let id = Uuid::new_v4();
        let expected = make_job("a.log", 1000, 500);
        let file_json = serde_json::to_string(abs_path("data/a.log").to_str().unwrap()).unwrap();
        let raw = format!(
            concat!(
                r#"{{"entries":[{{"id":"{id}","job":{{"#,
                r#""file":{file},"size":42,"#,
                r#""digest":"sha256:a.log","mtime":"1970-01-01T00:15:00Z","#,
                r#""first_observed_at":"1970-01-01T00:16:40Z","#,
                r#""last_observed_at":"1970-01-01T00:16:40Z","ttl_secs":500,"#,
                r#""file_rule_id":"file_rule_1","deployment_id":"dpl_1"}}}}]}}"#,
            ),
            id = id,
            file = file_json,
        );
        files::write_string(&path, &raw, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let queue = Queue::from_snapshot(8, open(&path).await, Persistence::BestEffort);

        assert_eq!(queue.len(), 1);
        let entry = queue.next_ready(now()).unwrap();
        assert_eq!(entry.id, id);
        assert_eq!(entry.job, expected);
    }

    /// Pins the expiry's wire format: the deleter's server deadline and saved
    /// remaining TTL ride under `expiry`, and a job without one (saved before
    /// expiries existed) omits the key, as the test above loads.
    #[test]
    fn expiry_round_trips_under_its_own_key() {
        let job = Job {
            expiry: Expiry {
                server_expires_at: Some(DateTime::from_timestamp(1500, 0).unwrap()),
                remaining_ttl_secs: Some(300),
            },
            ..make_job("a.log", 1000, 500)
        };

        let value = serde_json::to_value(&job).unwrap();
        assert_eq!(
            value["expiry"],
            serde_json::json!({
                "server_expires_at": "1970-01-01T00:25:00Z",
                "remaining_ttl_secs": 300,
            })
        );
        assert_eq!(serde_json::from_value::<Job>(value).unwrap(), job);

        let unset = serde_json::to_value(make_job("a.log", 1000, 500)).unwrap();
        assert!(unset.get("expiry").is_none());
    }
}
