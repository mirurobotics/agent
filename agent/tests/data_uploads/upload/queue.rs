//! The upload queue: the generic suite instantiated for `upload::Job`, plus
//! the on-disk wire pin for `upload_queue.json`.
//!
//! Queue *behavior* is written once in `agent/tests/data_uploads/queue.rs` and
//! run here via `queue_suite!` against this worker's job type. What cannot live
//! there is the wire pin below: the generic suite is parameterized over the job
//! type and so cannot spell a concrete payload, but the bytes on disk are a
//! per-worker artifact — and this file is released user data, so a shape change
//! would wipe a real queue rather than fail.

// internal crates
use crate::data_uploads::queue::queue_suite;
use miru_agent::data_uploads::queue::QueueJob;
use miru_agent::data_uploads::upload::{Job, Queue, QueueEntry, QueueSnapshot, QueueSnapshotFile};
use miru_agent::filesys::{dirs, files, File, WriteOptions};

// external crates
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// A deterministic upload job. Upload jobs are due the moment they arrive, so
/// every one of these is due at `queue::now()`.
fn upload_job(name: &str) -> Job {
    Job {
        file: File::new(format!("/data/{name}")),
        size: 42,
        digest: format!("sha256:{name}"),
        mtime: DateTime::from_timestamp(900, 0).unwrap(),
        first_observed_at: DateTime::from_timestamp(1000, 0).unwrap(),
        last_observed_at: DateTime::from_timestamp(1000, 0).unwrap(),
        file_rule_id: "rule_1".to_string(),
        deployment_id: "dpl_1".to_string(),
        retention: None,
    }
}

queue_suite!(upload_job, "upload-generic-queue-test");

/// An upload job is due the moment it arrives: `due_at` is `MIN_UTC`, so
/// readiness is decided by `next_attempt_at` alone.
#[test]
fn due_at_is_min_utc() {
    assert_eq!(
        QueueJob::due_at(&upload_job("a.log")),
        DateTime::<Utc>::MIN_UTC
    );
}

fn make_job(name: &str) -> Job {
    Job {
        file: File::new(format!("/data/{name}")),
        size: 42,
        digest: format!("sha256:{name}"),
        mtime: Utc::now(),
        first_observed_at: Utc::now(),
        last_observed_at: Utc::now(),
        file_rule_id: "rule_1".to_string(),
        deployment_id: "dpl_1".to_string(),
        retention: None,
    }
}

/// A fresh snapshot file over `path`. Reopening the same path returns a
/// handle whose in-memory cache reflects what was previously persisted.
async fn open(path: &File) -> QueueSnapshotFile {
    QueueSnapshotFile::new_with_default(path.clone(), QueueSnapshot::default())
        .await
        .unwrap()
}

mod wire {
    use super::*;

    /// Pins the persisted wire format on the read side: an entry is
    /// `{id, job, attempts, next_attempt_at}` with the job's fields nested
    /// rather than flattened. `SingleThreadStateFile::new_with_default`
    /// silently overwrites a snapshot it cannot parse, so a shape change would
    /// wipe a live user's queue instead of erroring — this test is the guard.
    ///
    /// `make_job` stamps a fresh `Utc::now()`, so the expected job is spelled
    /// out here to match the literal JSON exactly.
    #[tokio::test]
    async fn raw_json_snapshot_loads() {
        let dir = dirs::temp("upload_queue_test").unwrap();
        let path = dir.to_dir().file("upload_queue.json");
        let id = Uuid::new_v4();
        let raw = format!(
            concat!(
                r#"{{"entries":[{{"id":"{id}","job":{{"#,
                r#""file":"/data/a.log","size":42,"#,
                r#""digest":"sha256:a.log","mtime":"1970-01-01T00:15:00Z","#,
                r#""first_observed_at":"1970-01-01T00:16:40Z","#,
                r#""last_observed_at":"1970-01-01T00:16:40Z","#,
                r#""file_rule_id":"rule_1","deployment_id":"dpl_1","#,
                r#""retention":null}},"attempts":0,"next_attempt_at":null}}]}}"#,
            ),
            id = id,
        );
        files::write_string(&path, &raw, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let queue = Queue::from_snapshot(8, open(&path).await);

        assert_eq!(queue.len(), 1);
        let entry = queue.next_ready(Utc::now()).unwrap();
        assert_eq!(entry.id, id);
        assert_eq!(
            entry.job,
            Job {
                file: File::new("/data/a.log".to_string()),
                size: 42,
                digest: "sha256:a.log".to_string(),
                mtime: DateTime::from_timestamp(900, 0).unwrap(),
                first_observed_at: DateTime::from_timestamp(1000, 0).unwrap(),
                last_observed_at: DateTime::from_timestamp(1000, 0).unwrap(),
                file_rule_id: "rule_1".to_string(),
                deployment_id: "dpl_1".to_string(),
                retention: None,
            }
        );
    }

    /// Pins the persisted wire format on the write side: the exact key set at
    /// each level, with `job` nested as an object rather than flattened into
    /// the entry. A rename, a reorder into a flattened shape, an enum wrapper,
    /// or an added `version` field all fail here.
    #[tokio::test]
    async fn enqueue_writes_the_pinned_json_shape() {
        let dir = dirs::temp("upload_queue_test").unwrap();
        let path = dir.to_dir().file("upload_queue.json");

        {
            let mut queue = Queue::from_snapshot(8, open(&path).await);
            queue.enqueue(make_job("a.log")).await.unwrap();
        }

        let raw = files::read_string(&path).await.unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();

        let top_keys: Vec<&str> = value
            .as_object()
            .unwrap_or_else(|| panic!("snapshot is not an object: {raw}"))
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(top_keys, ["entries"], "{raw}");

        let entries = value["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1, "{raw}");
        let entry = entries[0]
            .as_object()
            .unwrap_or_else(|| panic!("entry is not an object: {raw}"));
        let mut keys: Vec<&str> = entry.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["attempts", "id", "job", "next_attempt_at"], "{raw}");

        // the job stays a nested object; it is never flattened into the entry
        assert!(entry["job"].is_object(), "job is not nested: {raw}");
        assert_eq!(entry["job"]["digest"], "sha256:a.log", "{raw}");
        assert!(
            Uuid::parse_str(entry["id"].as_str().unwrap()).is_ok(),
            "id is not a uuid string: {raw}"
        );
        assert_eq!(entry["attempts"], 0, "{raw}");
        assert_eq!(entry["next_attempt_at"], serde_json::Value::Null, "{raw}");
    }

    #[tokio::test]
    async fn legacy_snapshot_without_next_attempt_at_loads() {
        let dir = dirs::temp("upload_queue_test").unwrap();
        let path = dir.to_dir().file("upload_queue.json");
        let snapshot = QueueSnapshot {
            entries: vec![QueueEntry {
                id: Uuid::new_v4(),
                job: make_job("a.log"),
                attempts: 2,
                next_attempt_at: None,
            }],
        };

        // strip the field a pre-backoff agent never wrote
        let mut value = serde_json::to_value(&snapshot).unwrap();
        for entry in value["entries"].as_array_mut().unwrap() {
            entry.as_object_mut().unwrap().remove("next_attempt_at");
        }
        files::write_string(&path, &value.to_string(), WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        // if deserialization failed, new_with_default would silently write an
        // empty default snapshot and the pop below would find nothing
        let queue = Queue::from_snapshot(8, open(&path).await);
        let entry = queue.next_ready(Utc::now()).unwrap();
        assert_eq!(entry.attempts, 2);
        assert_eq!(entry.next_attempt_at, None);
    }
}
