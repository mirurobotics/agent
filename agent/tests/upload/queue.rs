// internal crates
use miru_agent::filesys::{self, dirs, files, File, WriteOptions};
use miru_agent::upload::{Outcome, PendingJob, UploadErr, Job, Queue};

// external crates
use chrono::{DateTime, Duration, Utc};

async fn make_file(dir: &filesys::Dir, name: &str, contents: &str) -> File {
    let file = dir.file(name);
    files::write_string(&file, contents, WriteOptions::OVERWRITE_ATOMIC)
        .await
        .unwrap();
    file
}

async fn make_job(file: &File) -> Job {
    Job {
        file: file.clone(),
        size: files::size(file).await.unwrap(),
        digest: files::hash(file).await.unwrap(),
        mtime: DateTime::<Utc>::from(files::last_modified(file).await.unwrap()),
        upload_rule_id: "rule_1".to_string(),
        deployment_id: "dpl_1".to_string(),
        release_id: "rls_1".to_string(),
    }
}

mod enqueue {
    use super::*;

    #[tokio::test]
    async fn returns_enqueued_for_new_job() {
        let dir = dirs::create_temp("upload_queue_new").await.unwrap();
        let file = make_file(&dir, "a.log", "contents a").await;
        let mut queue = Queue::new(4);

        let outcome = queue.enqueue(make_job(&file).await).await.unwrap();

        assert_eq!(outcome, Outcome::Enqueued);
        assert_eq!(queue.len(), 1);
    }

    #[tokio::test]
    async fn returns_duplicate_for_same_key() {
        let dir = dirs::create_temp("upload_queue_dup").await.unwrap();
        let file = make_file(&dir, "a.log", "contents a").await;
        let mut queue = Queue::new(4);
        let job = make_job(&file).await;
        queue.enqueue(job.clone()).await.unwrap();

        let mut dup = job.clone();
        dup.deployment_id = "dpl_2".to_string();
        dup.release_id = "rls_2".to_string();
        let outcome = queue.enqueue(dup).await.unwrap();

        assert_eq!(outcome, Outcome::Duplicate);
        assert_eq!(queue.len(), 1);
    }

    #[tokio::test]
    async fn full_queue_with_stale_job_prunes_and_accepts() {
        let dir = dirs::create_temp("upload_queue_prune").await.unwrap();
        let fresh_file = make_file(&dir, "fresh.log", "fresh contents").await;
        let stale_file = make_file(&dir, "stale.log", "stale contents").await;
        let mut queue = Queue::new(2);
        let stale_job = make_job(&stale_file).await;
        let stale_key = stale_job.dedup_key();
        queue.enqueue(make_job(&fresh_file).await).await.unwrap();
        queue.enqueue(stale_job).await.unwrap();
        files::delete(&stale_file).await.unwrap();

        let new_file = make_file(&dir, "new.log", "new contents").await;
        let outcome = queue.enqueue(make_job(&new_file).await).await.unwrap();

        assert_eq!(outcome, Outcome::Enqueued);
        assert_eq!(queue.len(), 2);
        assert!(!queue.contains(&stale_key));
    }

    #[tokio::test]
    async fn full_queue_of_fresh_jobs_returns_queue_full_err() {
        let dir = dirs::create_temp("upload_queue_full").await.unwrap();
        let file_a = make_file(&dir, "a.log", "contents a").await;
        let file_b = make_file(&dir, "b.log", "contents b").await;
        let mut queue = Queue::new(1);
        queue.enqueue(make_job(&file_a).await).await.unwrap();

        let result = queue.enqueue(make_job(&file_b).await).await;

        assert!(
            matches!(result, Err(UploadErr::QueueFullErr(_))),
            "expected QueueFullErr, got: {result:?}"
        );
        assert_eq!(queue.len(), 1);
    }

    #[tokio::test]
    async fn changed_content_makes_job_stale() {
        let dir = dirs::create_temp("upload_queue_stale_content")
            .await
            .unwrap();
        let file = make_file(&dir, "a.log", "original").await;
        let mut queue = Queue::new(1);
        let job = make_job(&file).await;
        let key = job.dedup_key();
        queue.enqueue(job).await.unwrap();
        files::write_string(
            &file,
            "rewritten with different size",
            WriteOptions::OVERWRITE_ATOMIC,
        )
        .await
        .unwrap();

        let other = make_file(&dir, "b.log", "other contents").await;
        let outcome = queue.enqueue(make_job(&other).await).await.unwrap();

        assert_eq!(outcome, Outcome::Enqueued);
        assert_eq!(queue.len(), 1);
        assert!(!queue.contains(&key));
    }

    #[tokio::test]
    async fn changed_mtime_makes_job_stale() {
        let dir = dirs::create_temp("upload_queue_stale_mtime").await.unwrap();
        let file = make_file(&dir, "a.log", "contents a").await;
        let mut queue = Queue::new(1);
        let mut job = make_job(&file).await;
        // lie about the recorded mtime so the file appears rewritten
        job.mtime += Duration::seconds(1);
        let key = job.dedup_key();
        queue.enqueue(job).await.unwrap();

        let other = make_file(&dir, "b.log", "other contents").await;
        let outcome = queue.enqueue(make_job(&other).await).await.unwrap();

        assert_eq!(outcome, Outcome::Enqueued);
        assert_eq!(queue.len(), 1);
        assert!(!queue.contains(&key));
    }

    #[tokio::test]
    async fn missing_file_makes_job_stale() {
        let dir = dirs::create_temp("upload_queue_stale_missing")
            .await
            .unwrap();
        let file = make_file(&dir, "a.log", "original").await;
        let mut queue = Queue::new(1);
        let job = make_job(&file).await;
        let key = job.dedup_key();
        queue.enqueue(job).await.unwrap();
        files::delete(&file).await.unwrap();

        let other = make_file(&dir, "b.log", "other contents").await;
        let outcome = queue.enqueue(make_job(&other).await).await.unwrap();

        assert_eq!(outcome, Outcome::Enqueued);
        assert_eq!(queue.len(), 1);
        assert!(!queue.contains(&key));
    }
}

mod is_empty {
    use super::*;

    #[tokio::test]
    async fn reflects_queue_contents() {
        let dir = dirs::create_temp("upload_queue_is_empty").await.unwrap();
        let file = make_file(&dir, "a.log", "contents a").await;
        let mut queue = Queue::new(4);
        assert!(queue.is_empty());

        queue.enqueue(make_job(&file).await).await.unwrap();
        assert!(!queue.is_empty());
    }
}

mod pop_front {
    use super::*;

    #[tokio::test]
    async fn returns_jobs_in_fifo_order() {
        let dir = dirs::create_temp("upload_queue_fifo").await.unwrap();
        let mut queue = Queue::new(4);
        let mut jobs = Vec::new();
        for name in ["a.log", "b.log", "c.log"] {
            let file = make_file(&dir, name, name).await;
            let job = make_job(&file).await;
            queue.enqueue(job.clone()).await.unwrap();
            jobs.push(job);
        }

        for expected in jobs {
            let pending = queue.pop_front().unwrap();
            assert_eq!(pending.job, expected);
            assert_eq!(pending.attempts, 0);
        }
        assert!(queue.pop_front().is_none());
    }
}

mod requeue {
    use super::*;

    #[tokio::test]
    async fn preserves_attempts_and_appends_at_tail() {
        let dir = dirs::create_temp("upload_queue_requeue").await.unwrap();
        let file_a = make_file(&dir, "a.log", "contents a").await;
        let file_b = make_file(&dir, "b.log", "contents b").await;
        let mut queue = Queue::new(4);
        let job_a = make_job(&file_a).await;
        queue.enqueue(job_a.clone()).await.unwrap();
        let requeued_job = make_job(&file_b).await;
        queue
            .requeue(PendingJob {
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
}
