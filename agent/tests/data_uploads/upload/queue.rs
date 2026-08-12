// internal crates
use miru_agent::data_uploads::upload::{
    Job, Queue, QueueEntry, QueueSnapshot, QueueSnapshotFile, UploadErr,
};
use miru_agent::filesys::{dirs, files, File, WriteOptions};

// external crates
use chrono::{DateTime, TimeDelta, Utc};

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

/// The deterministic digests of a queue's jobs, in order — a stable identity
/// for FIFO assertions (`make_job` stamps a fresh `Utc::now()` each call, so
/// whole-`Job` equality across a reload does not hold).
async fn digests(queue: &mut Queue) -> Vec<String> {
    let mut out = Vec::new();
    while let Some(entry) = queue.pop_ready(Utc::now()).await {
        out.push(entry.job.digest);
    }
    out
}

mod from_snapshot {
    use super::*;

    #[tokio::test]
    async fn empty_snapshot_loads_empty_queue() {
        let dir = dirs::temp("upload_queue_test").unwrap();
        let path = dir.to_dir().file("upload_queue.json");

        let queue = Queue::from_snapshot(8, open(&path).await);

        assert!(queue.is_empty());
    }

    #[tokio::test]
    async fn legacy_snapshot_without_next_attempt_at_loads() {
        let dir = dirs::temp("upload_queue_test").unwrap();
        let path = dir.to_dir().file("upload_queue.json");
        let snapshot = QueueSnapshot {
            entries: vec![QueueEntry {
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
        let mut queue = Queue::from_snapshot(8, open(&path).await);
        let entry = queue.pop_ready(Utc::now()).await.unwrap();
        assert_eq!(entry.attempts, 2);
        assert_eq!(entry.next_attempt_at, None);
    }
}

mod enqueue {
    use super::*;

    #[tokio::test]
    async fn appends_new_job() {
        let mut queue = Queue::new(4);

        queue.enqueue(make_job("a.log")).await.unwrap();

        assert_eq!(queue.len(), 1);
    }

    #[tokio::test]
    async fn survives_reopen_in_fifo_order() {
        let dir = dirs::temp("upload_queue_test").unwrap();
        let path = dir.to_dir().file("upload_queue.json");

        {
            let mut queue = Queue::from_snapshot(8, open(&path).await);
            queue.enqueue(make_job("a.log")).await.unwrap();
            queue.enqueue(make_job("b.log")).await.unwrap();
        }

        let mut reloaded = Queue::from_snapshot(8, open(&path).await);
        assert_eq!(reloaded.len(), 2);
        assert_eq!(
            digests(&mut reloaded).await,
            vec!["sha256:a.log".to_string(), "sha256:b.log".to_string()]
        );
    }

    #[tokio::test]
    async fn duplicate_jobs_are_both_queued() {
        let mut queue = Queue::new(4);
        let job = make_job("a.log");

        queue.enqueue(job.clone()).await.unwrap();
        queue.enqueue(job).await.unwrap();

        assert_eq!(queue.len(), 2);
    }

    #[tokio::test]
    async fn full_queue_returns_queue_full_err() {
        let mut queue = Queue::new(1);
        queue.enqueue(make_job("a.log")).await.unwrap();

        let err = queue.enqueue(make_job("b.log")).await.unwrap_err();

        assert!(
            matches!(err, UploadErr::QueueFullErr(_)),
            "expected QueueFullErr, got: {err:?}"
        );
        assert!(err.to_string().contains("queue is full"), "message: {err}");
        assert_eq!(queue.len(), 1);
    }

    #[tokio::test]
    async fn rejection_does_not_persist() {
        let dir = dirs::temp("upload_queue_test").unwrap();
        let path = dir.to_dir().file("upload_queue.json");

        {
            let mut queue = Queue::from_snapshot(1, open(&path).await);
            queue.enqueue(make_job("a.log")).await.unwrap();
            queue.enqueue(make_job("b.log")).await.unwrap_err();
        }

        let mut reloaded = Queue::from_snapshot(1, open(&path).await);
        assert_eq!(
            digests(&mut reloaded).await,
            vec!["sha256:a.log".to_string()]
        );
    }

    #[tokio::test]
    async fn persist_failure_is_swallowed() {
        use std::os::unix::fs::PermissionsExt;

        let dir = dirs::temp("upload_queue_test").unwrap();
        let locked = dir.to_dir().subdir("locked");
        // new_with_default creates `locked` and the file inside it.
        let file = open(&locked.file("upload_queue.json")).await;
        let mut queue = Queue::from_snapshot(4, file);

        // Make the snapshot's directory read-only so the atomic write fails.
        dirs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555))
            .await
            .unwrap();

        // The enqueue reports success despite the underlying persist error.
        queue.enqueue(make_job("a.log")).await.unwrap();
        assert_eq!(queue.len(), 1);

        // Restore permissions so the tempdir can clean itself up on drop.
        dirs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755))
            .await
            .unwrap();
    }
}

mod is_empty {
    use super::*;

    #[tokio::test]
    async fn reflects_queue_contents() {
        let mut queue = Queue::new(4);
        assert!(queue.is_empty());

        queue.enqueue(make_job("a.log")).await.unwrap();
        assert!(!queue.is_empty());
    }
}

mod requeue {
    use super::*;

    #[tokio::test]
    async fn preserves_attempts_and_appends_at_tail() {
        let mut queue = Queue::new(4);
        let job_a = make_job("a.log");
        queue.enqueue(job_a.clone()).await.unwrap();
        let requeued_job = make_job("b.log");
        queue
            .requeue(QueueEntry {
                job: requeued_job.clone(),
                attempts: 3,
                next_attempt_at: None,
            })
            .await;

        let first = queue.pop_ready(Utc::now()).await.unwrap();
        assert_eq!(first.job, job_a);
        let second = queue.pop_ready(Utc::now()).await.unwrap();
        assert_eq!(second.job, requeued_job);
        assert_eq!(second.attempts, 3);
    }

    #[tokio::test]
    async fn persists_attempts() {
        let dir = dirs::temp("upload_queue_test").unwrap();
        let path = dir.to_dir().file("upload_queue.json");

        {
            let mut queue = Queue::from_snapshot(8, open(&path).await);
            queue
                .requeue(QueueEntry {
                    job: make_job("a.log"),
                    attempts: 5,
                    next_attempt_at: None,
                })
                .await;
        }

        let mut reloaded = Queue::from_snapshot(8, open(&path).await);
        let entry = reloaded.pop_ready(Utc::now()).await.unwrap();
        assert_eq!(entry.job.digest, "sha256:a.log");
        assert_eq!(entry.attempts, 5);
    }

    #[tokio::test]
    async fn next_attempt_at_survives_reload() {
        let dir = dirs::temp("upload_queue_test").unwrap();
        let path = dir.to_dir().file("upload_queue.json");
        let deadline = DateTime::from_timestamp(2_000_000_000, 0).unwrap();

        {
            let mut queue = Queue::from_snapshot(8, open(&path).await);
            queue
                .requeue(QueueEntry {
                    job: make_job("a.log"),
                    attempts: 5,
                    next_attempt_at: Some(deadline),
                })
                .await;
        }

        let mut reloaded = Queue::from_snapshot(8, open(&path).await);
        let entry = reloaded.pop_ready(deadline).await.unwrap();
        assert_eq!(entry.job.digest, "sha256:a.log");
        assert_eq!(entry.attempts, 5);
        assert_eq!(entry.next_attempt_at, Some(deadline));
    }

    #[tokio::test]
    async fn full_queue_still_accepts_requeue() {
        let mut queue = Queue::new(1);
        queue.enqueue(make_job("a.log")).await.unwrap();

        queue
            .requeue(QueueEntry {
                job: make_job("b.log"),
                attempts: 2,
                next_attempt_at: None,
            })
            .await;

        assert_eq!(queue.len(), 2);
        assert_eq!(
            digests(&mut queue).await,
            vec!["sha256:a.log".to_string(), "sha256:b.log".to_string()]
        );
    }
}

mod pop_ready {
    use super::*;

    #[tokio::test]
    async fn returns_jobs_in_fifo_order() {
        let mut queue = Queue::new(4);
        let mut jobs = Vec::new();
        for name in ["a.log", "b.log", "c.log"] {
            let job = make_job(name);
            queue.enqueue(job.clone()).await.unwrap();
            jobs.push(job);
        }

        for expected in jobs {
            let entry = queue.pop_ready(Utc::now()).await.unwrap();
            assert_eq!(entry.job, expected);
            assert_eq!(entry.attempts, 0);
        }
        assert!(queue.pop_ready(Utc::now()).await.is_none());
    }

    #[tokio::test]
    async fn skips_waiting_entries() {
        let mut queue = Queue::new(4);
        let now = Utc::now();
        let deadline = now + TimeDelta::hours(1);
        queue
            .requeue(QueueEntry {
                job: make_job("a.log"),
                attempts: 1,
                next_attempt_at: Some(deadline),
            })
            .await;
        queue.enqueue(make_job("b.log")).await.unwrap();
        queue.enqueue(make_job("c.log")).await.unwrap();

        // waiting A is skipped; eligible entries still pop in FIFO order
        assert_eq!(
            queue.pop_ready(now).await.unwrap().job.digest,
            "sha256:b.log"
        );
        assert_eq!(
            queue.pop_ready(now).await.unwrap().job.digest,
            "sha256:c.log"
        );
        // the deadline itself is eligible: the comparison is inclusive
        let entry = queue.pop_ready(deadline).await.unwrap();
        assert_eq!(entry.job.digest, "sha256:a.log");
        assert_eq!(entry.attempts, 1);
    }

    #[tokio::test]
    async fn returns_none_when_all_waiting() {
        let mut queue = Queue::new(4);
        let now = Utc::now();
        queue
            .requeue(QueueEntry {
                job: make_job("a.log"),
                attempts: 1,
                next_attempt_at: Some(now + TimeDelta::hours(1)),
            })
            .await;

        assert!(queue.pop_ready(now).await.is_none());
        assert_eq!(queue.len(), 1);
    }

    #[tokio::test]
    async fn persists_shorter_backlog() {
        let dir = dirs::temp("upload_queue_test").unwrap();
        let path = dir.to_dir().file("upload_queue.json");

        {
            let mut queue = Queue::from_snapshot(8, open(&path).await);
            queue.enqueue(make_job("a.log")).await.unwrap();
            queue.enqueue(make_job("b.log")).await.unwrap();
            queue.pop_ready(Utc::now()).await.unwrap();
        }

        let mut reloaded = Queue::from_snapshot(8, open(&path).await);
        assert_eq!(
            digests(&mut reloaded).await,
            vec!["sha256:b.log".to_string()]
        );
    }
}

mod earliest_next_attempt {
    use super::*;

    #[test]
    fn none_when_empty() {
        assert_eq!(Queue::new(4).earliest_next_attempt(), None);
    }

    #[tokio::test]
    async fn returns_min_deadline() {
        let mut queue = Queue::new(4);
        let t1 = DateTime::from_timestamp(1_000_000_000, 0).unwrap();
        let t2 = DateTime::from_timestamp(1_500_000_000, 0).unwrap();
        for (name, deadline) in [("a.log", t2), ("b.log", t1)] {
            queue
                .requeue(QueueEntry {
                    job: make_job(name),
                    attempts: 1,
                    next_attempt_at: Some(deadline),
                })
                .await;
        }

        assert_eq!(queue.earliest_next_attempt(), Some(t1));
    }

    #[tokio::test]
    async fn none_deadline_counts_as_min_utc() {
        let mut queue = Queue::new(4);
        queue
            .requeue(QueueEntry {
                job: make_job("a.log"),
                attempts: 1,
                next_attempt_at: Some(Utc::now() + TimeDelta::hours(1)),
            })
            .await;
        queue.enqueue(make_job("b.log")).await.unwrap();

        assert_eq!(
            queue.earliest_next_attempt(),
            Some(DateTime::<Utc>::MIN_UTC)
        );
    }
}
