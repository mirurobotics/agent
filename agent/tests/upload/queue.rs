// internal crates
use miru_agent::filesys::File;
use miru_agent::upload::{Job, Queue, QueueEntry, UploadErr};

// external crates
use chrono::Utc;

fn make_job(name: &str) -> Job {
    Job {
        file: File::new(format!("/data/{name}")),
        size: 42,
        digest: format!("sha256:{name}"),
        crc32c: None,
        mtime: Utc::now(),
        first_observed_at: Utc::now(),
        last_observed_at: Utc::now(),
        upload_rule_id: "rule_1".to_string(),
        deployment_id: "dpl_1".to_string(),
        release_id: "rls_1".to_string(),
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

        let first = queue.pop_front().unwrap();
        assert_eq!(first.job, job_a);
        let second = queue.pop_front().unwrap();
        assert_eq!(second.job, requeued_job);
        assert_eq!(second.attempts, 3);
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
            let entry = queue.pop_front().unwrap();
            assert_eq!(entry.job, expected);
            assert_eq!(entry.attempts, 0);
        }
        assert!(queue.pop_front().is_none());
    }
}
