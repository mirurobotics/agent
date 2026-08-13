// internal crates
use miru_agent::data_uploads::upload::{
    Job, Queue, QueueEntry, QueueSnapshot, QueueSnapshotFile, UploadErr,
};
use miru_agent::filesys::{dirs, files, File, WriteOptions};

// external crates
use chrono::{DateTime, TimeDelta, Utc};
use uuid::Uuid;

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
    // remove as the worker does: next_ready leaves the entry in place, so
    // draining without removing would return the same entry forever
    while let Some(entry) = queue.next_ready(Utc::now()) {
        out.push(entry.job.digest);
        queue.remove(entry.id).await;
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
                id: Uuid::new_v4(),
                job: requeued_job.clone(),
                attempts: 3,
                next_attempt_at: None,
            })
            .await;

        let first = queue.next_ready(Utc::now()).unwrap();
        assert_eq!(first.job, job_a);
        queue.remove(first.id).await;
        let second = queue.next_ready(Utc::now()).unwrap();
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
                    id: Uuid::new_v4(),
                    job: make_job("a.log"),
                    attempts: 5,
                    next_attempt_at: None,
                })
                .await;
        }

        let reloaded = Queue::from_snapshot(8, open(&path).await);
        let entry = reloaded.next_ready(Utc::now()).unwrap();
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
                    id: Uuid::new_v4(),
                    job: make_job("a.log"),
                    attempts: 5,
                    next_attempt_at: Some(deadline),
                })
                .await;
        }

        let reloaded = Queue::from_snapshot(8, open(&path).await);
        let entry = reloaded.next_ready(deadline).unwrap();
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
                id: Uuid::new_v4(),
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

    /// Requeuing an entry the queue already holds moves it, rather than
    /// leaving a stale copy behind under the same id.
    #[tokio::test]
    async fn replaces_the_entry_with_the_same_id() {
        let mut queue = Queue::new(4);
        queue.enqueue(make_job("a.log")).await.unwrap();
        let entry = queue.next_ready(Utc::now()).unwrap();

        queue
            .requeue(QueueEntry {
                attempts: 1,
                ..entry
            })
            .await;

        assert_eq!(queue.len(), 1);
        assert_eq!(queue.next_ready(Utc::now()).unwrap().attempts, 1);
    }
}

mod next_ready {
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
            let entry = queue.next_ready(Utc::now()).unwrap();
            assert_eq!(entry.job, expected);
            assert_eq!(entry.attempts, 0);
            queue.remove(entry.id).await;
        }
        assert!(queue.next_ready(Utc::now()).is_none());
    }

    #[tokio::test]
    async fn skips_waiting_entries() {
        let mut queue = Queue::new(4);
        let now = Utc::now();
        let deadline = now + TimeDelta::hours(1);
        queue
            .requeue(QueueEntry {
                id: Uuid::new_v4(),
                job: make_job("a.log"),
                attempts: 1,
                next_attempt_at: Some(deadline),
            })
            .await;
        queue.enqueue(make_job("b.log")).await.unwrap();
        queue.enqueue(make_job("c.log")).await.unwrap();

        // waiting A is skipped; eligible entries still pop in FIFO order
        let b = queue.next_ready(now).unwrap();
        assert_eq!(b.job.digest, "sha256:b.log");
        queue.remove(b.id).await;
        let c = queue.next_ready(now).unwrap();
        assert_eq!(c.job.digest, "sha256:c.log");
        queue.remove(c.id).await;
        // the deadline itself is eligible: the comparison is inclusive
        let entry = queue.next_ready(deadline).unwrap();
        assert_eq!(entry.job.digest, "sha256:a.log");
        assert_eq!(entry.attempts, 1);
    }

    #[tokio::test]
    async fn returns_none_when_all_waiting() {
        let mut queue = Queue::new(4);
        let now = Utc::now();
        queue
            .requeue(QueueEntry {
                id: Uuid::new_v4(),
                job: make_job("a.log"),
                attempts: 1,
                next_attempt_at: Some(now + TimeDelta::hours(1)),
            })
            .await;

        assert!(queue.next_ready(now).is_none());
        assert_eq!(queue.len(), 1);
    }

    #[tokio::test]
    async fn leaves_the_entry_on_disk_until_removed() {
        let dir = dirs::temp("upload_queue_test").unwrap();
        let path = dir.to_dir().file("upload_queue.json");

        {
            let mut queue = Queue::from_snapshot(8, open(&path).await);
            queue.enqueue(make_job("a.log")).await.unwrap();
            queue.enqueue(make_job("b.log")).await.unwrap();
            queue.next_ready(Utc::now()).unwrap();
        }

        let mut reloaded = Queue::from_snapshot(8, open(&path).await);
        assert_eq!(
            digests(&mut reloaded).await,
            vec!["sha256:a.log".to_string(), "sha256:b.log".to_string()]
        );
    }
}

mod remove {
    use super::*;

    #[tokio::test]
    async fn removes_the_entry_from_disk() {
        let dir = dirs::temp("upload_queue_test").unwrap();
        let path = dir.to_dir().file("upload_queue.json");

        {
            let mut queue = Queue::from_snapshot(8, open(&path).await);
            queue.enqueue(make_job("a.log")).await.unwrap();
            queue.enqueue(make_job("b.log")).await.unwrap();
            let entry = queue.next_ready(Utc::now()).unwrap();
            let removed = queue.remove(entry.id).await.unwrap();
            assert_eq!(removed.id, entry.id);
        }

        let mut reloaded = Queue::from_snapshot(8, open(&path).await);
        assert_eq!(
            digests(&mut reloaded).await,
            vec!["sha256:b.log".to_string()]
        );
    }

    #[tokio::test]
    async fn unknown_id_is_ignored() {
        let mut queue = Queue::new(4);
        queue.enqueue(make_job("a.log")).await.unwrap();

        assert!(queue.remove(Uuid::new_v4()).await.is_none());
        assert_eq!(queue.len(), 1);
    }

    /// Identity is per-entry, not per-job: enqueuing the same file twice
    /// leaves two entries that are removable independently.
    #[tokio::test]
    async fn removing_one_duplicate_leaves_the_other() {
        let mut queue = Queue::new(4);
        let job = make_job("a.log");
        queue.enqueue(job.clone()).await.unwrap();
        queue.enqueue(job).await.unwrap();

        let entry = queue.next_ready(Utc::now()).unwrap();
        queue.remove(entry.id).await.unwrap();

        assert_eq!(queue.len(), 1);
        assert_ne!(queue.next_ready(Utc::now()).unwrap().id, entry.id);
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
                    id: Uuid::new_v4(),
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
                id: Uuid::new_v4(),
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

mod reset_invalid_deadlines {
    use super::*;

    #[tokio::test]
    async fn invalid_deadline_is_pulled_back_to_the_horizon() {
        let mut queue = Queue::new(4);
        let now = Utc::now();
        let horizon = now + TimeDelta::hours(24);
        // no 24h-max backoff schedule could have stamped this: it is what a
        // snapshot written before a backward clock step looks like
        queue
            .requeue(QueueEntry {
                id: Uuid::new_v4(),
                job: make_job("a.log"),
                attempts: 3,
                next_attempt_at: Some(now + TimeDelta::hours(48)),
            })
            .await;

        queue.reset_invalid_deadlines(horizon);

        // stranded for 48h before the reset; now bounded by the horizon rather
        // than made instantly eligible, so a clock still settling after boot is
        // not hammered
        assert!(queue.next_ready(now).is_none());
        let entry = queue.next_ready(horizon).unwrap();
        assert_eq!(entry.next_attempt_at, Some(horizon));
        // attempt count intact, so the retry budget still applies
        assert_eq!(entry.attempts, 3);
    }

    #[tokio::test]
    async fn only_deadlines_past_the_horizon_move() {
        let mut queue = Queue::new(8);
        let now = Utc::now();
        let horizon = now + TimeDelta::hours(24);
        let inside = horizon - TimeDelta::seconds(1);
        let beyond = horizon + TimeDelta::seconds(1);
        for (name, deadline) in [("a.log", inside), ("b.log", horizon), ("c.log", beyond)] {
            queue
                .requeue(QueueEntry {
                    id: Uuid::new_v4(),
                    job: make_job(name),
                    attempts: 1,
                    next_attempt_at: Some(deadline),
                })
                .await;
        }

        queue.reset_invalid_deadlines(horizon);

        // every entry is visited, and only the one strictly past the horizon
        // moves — the other two keep their own deadlines
        let mut drained = Vec::new();
        while let Some(entry) = queue.next_ready(beyond) {
            drained.push((entry.job.digest.clone(), entry.next_attempt_at));
            queue.remove(entry.id).await;
        }
        assert_eq!(
            drained,
            vec![
                ("sha256:a.log".to_string(), Some(inside)),
                ("sha256:b.log".to_string(), Some(horizon)),
                ("sha256:c.log".to_string(), Some(horizon)),
            ]
        );
    }

    #[tokio::test]
    async fn deadline_within_horizon_is_untouched() {
        let mut queue = Queue::new(4);
        let now = Utc::now();
        let horizon = now + TimeDelta::hours(24);
        // exactly at the horizon: an ordinary maximum-backoff wait, still honored
        queue
            .requeue(QueueEntry {
                id: Uuid::new_v4(),
                job: make_job("a.log"),
                attempts: 7,
                next_attempt_at: Some(horizon),
            })
            .await;

        queue.reset_invalid_deadlines(horizon);

        assert!(queue.next_ready(now).is_none());
        assert_eq!(queue.len(), 1);
    }

    #[tokio::test]
    async fn reset_is_in_memory_until_the_next_mutation() {
        let dir = dirs::temp("upload_queue_test").unwrap();
        let path = dir.to_dir().file("upload_queue.json");
        let now = Utc::now();
        let stale = now + TimeDelta::hours(48);

        let horizon = now + TimeDelta::hours(24);

        let mut queue = Queue::from_snapshot(8, open(&path).await);
        // the requeues persist the stale stamps; the reset deliberately does
        // not write, so the file still carries them afterwards
        for name in ["a.log", "b.log"] {
            queue
                .requeue(QueueEntry {
                    id: Uuid::new_v4(),
                    job: make_job(name),
                    attempts: 2,
                    next_attempt_at: Some(stale),
                })
                .await;
        }

        queue.reset_invalid_deadlines(horizon);

        let head_deadline = |raw: &str| -> serde_json::Value {
            serde_json::from_str::<serde_json::Value>(raw).unwrap()["entries"][0]["next_attempt_at"]
                .clone()
        };
        let raw = files::read_string(&path).await.unwrap();
        assert_eq!(
            head_deadline(&raw),
            serde_json::to_value(stale).unwrap(),
            "the reset should not have rewritten the snapshot: {raw}"
        );

        // the next mutation writes the corrected entries through
        let entry = queue.next_ready(horizon).unwrap();
        queue.remove(entry.id).await;
        let raw = files::read_string(&path).await.unwrap();
        assert_eq!(
            head_deadline(&raw),
            serde_json::to_value(horizon).unwrap(),
            "the removal should have persisted the pulled-back deadline: {raw}"
        );
    }
}
