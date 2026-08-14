//! The on-disk wire pin for `delete_queue.json`.
//!
//! Queue *behavior* is exercised in `agent/tests/data_uploads/queue.rs`, which
//! drives the one generic implementation against both production job types.
//! What cannot live there is this: the generic suite is parameterized over the
//! job type and so cannot spell a concrete payload, but the bytes on disk are
//! a per-worker artifact. `SingleThreadStateFile::new_with_default` falls back
//! to writing the default on any parse error, so a shape change would wipe a
//! live queue rather than fail — this pin is what turns that into a red test.

// internal crates
use miru_agent::data_uploads::retention::{
    DeleteQueueSnapshot, DeleteQueueSnapshotFile, Job, Queue,
};
use miru_agent::filesys::{dirs, files, File, WriteOptions};

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
    }
}

/// A fresh snapshot handle over `path`. Reopening the same path returns a
/// handle whose in-memory cache reflects what was previously persisted.
async fn open(path: &File) -> DeleteQueueSnapshotFile {
    DeleteQueueSnapshotFile::new_with_default(path.clone(), DeleteQueueSnapshot::default())
        .await
        .unwrap()
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
