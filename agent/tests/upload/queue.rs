// internal crates
use miru_agent::filesys::{dirs, File};
use miru_agent::upload::{Job, Queue, QueueEntry, QueueSnapshot, QueueSnapshotFile, UploadErr};

// external crates
use chrono::Utc;

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
    while let Some(entry) = queue.pop_front().await {
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
            })
            .await
            .unwrap();

        let first = queue.pop_front().await.unwrap();
        assert_eq!(first.job, job_a);
        let second = queue.pop_front().await.unwrap();
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
                })
                .await
                .unwrap();
        }

        let mut reloaded = Queue::from_snapshot(8, open(&path).await);
        let entry = reloaded.pop_front().await.unwrap();
        assert_eq!(entry.job.digest, "sha256:a.log");
        assert_eq!(entry.attempts, 5);
    }

    #[tokio::test]
    async fn full_queue_rejects_requeue() {
        // Requeue honors capacity too — it never evicts to make room.
        let mut queue = Queue::new(1);
        queue.enqueue(make_job("a.log")).await.unwrap();

        let result = queue
            .requeue(QueueEntry {
                job: make_job("b.log"),
                attempts: 2,
            })
            .await;

        assert!(
            matches!(result, Err(UploadErr::QueueFullErr(_))),
            "expected QueueFullErr, got: {result:?}"
        );
        assert_eq!(queue.len(), 1);
    }
}

mod pop_front {
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
            let entry = queue.pop_front().await.unwrap();
            assert_eq!(entry.job, expected);
            assert_eq!(entry.attempts, 0);
        }
        assert!(queue.pop_front().await.is_none());
    }

    #[tokio::test]
    async fn persists_shorter_backlog() {
        let dir = dirs::temp("upload_queue_test").unwrap();
        let path = dir.to_dir().file("upload_queue.json");

        {
            let mut queue = Queue::from_snapshot(8, open(&path).await);
            queue.enqueue(make_job("a.log")).await.unwrap();
            queue.enqueue(make_job("b.log")).await.unwrap();
            queue.pop_front().await.unwrap();
        }

        let mut reloaded = Queue::from_snapshot(8, open(&path).await);
        assert_eq!(
            digests(&mut reloaded).await,
            vec!["sha256:b.log".to_string()]
        );
    }
}
