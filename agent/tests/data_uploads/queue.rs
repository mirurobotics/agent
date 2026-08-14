//! The generic job queue, exercised against BOTH production job types.
//!
//! Every case is written once as a generic `check_*` helper over
//! `J: QueueJob` and then run twice — once for `upload::Job` (always due on
//! arrival) and once for `retention::Job` (due only once its TTL elapses) —
//! so the `due_at` hook is proven in both of its states.

// internal crates
use miru_agent::data_uploads::queue::{
    Queue, QueueEntry, QueueJob, QueueSnapshot, QueueSnapshotFile,
};
use miru_agent::data_uploads::retention;
use miru_agent::data_uploads::upload;
use miru_agent::filesys::{dirs, files, Dir, File, PathExt, WriteOptions};

// external crates
use chrono::{DateTime, TimeDelta, Utc};
use uuid::Uuid;

const DEFAULT_CAPACITY: usize = 4096;

/// The instant every test selects at. Both job factories below produce jobs
/// that are due at it.
fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(2000, 0).unwrap()
}

/// A deterministic upload job. Upload jobs are due the moment they arrive.
fn upload_job(name: &str) -> upload::Job {
    upload::Job {
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

/// A deterministic retention job with a zero TTL, so it is due at [`now`].
fn retention_job(name: &str) -> retention::Job {
    retention_job_with_ttl(name, 0)
}

fn retention_job_with_ttl(name: &str, ttl_secs: u64) -> retention::Job {
    retention::Job {
        file: File::new(format!("/data/{name}")),
        size: 42,
        digest: format!("sha256:{name}"),
        mtime: DateTime::from_timestamp(900, 0).unwrap(),
        first_observed_at: DateTime::from_timestamp(1000, 0).unwrap(),
        last_observed_at: DateTime::from_timestamp(1000, 0).unwrap(),
        ttl_secs,
        file_rule_id: "file_rule_1".to_string(),
        deployment_id: "dpl_1".to_string(),
    }
}

/// A fresh snapshot handle over `path`. Reopening the same path returns a
/// handle whose in-memory cache reflects what was previously persisted.
async fn open<J: QueueJob>(path: &File) -> QueueSnapshotFile<J> {
    QueueSnapshotFile::<J>::new_with_default(path.clone(), QueueSnapshot::<J>::default())
        .await
        .unwrap()
}

/// `enqueue` without requiring the error to be `Debug`.
async fn enqueue<J: QueueJob>(queue: &mut Queue<J>, job: J) {
    assert!(queue.enqueue(job).await.is_ok(), "enqueue was rejected");
}

/// The paths of the ready jobs, in selection order. `next_ready` leaves the
/// entry in place, so draining without removing would return the same entry
/// forever.
async fn drain<J: QueueJob>(queue: &mut Queue<J>) -> Vec<String> {
    let mut out = Vec::new();
    while let Some(entry) = queue.next_ready(now()) {
        out.push(entry.job.name());
        queue.remove(entry.id).await;
    }
    out
}

/// The paths persisted at `path`, in queue order. Non-destructive: it reads
/// the snapshot through a fresh handle rather than draining a queue.
async fn on_disk<J: QueueJob>(path: &File) -> Vec<String> {
    open::<J>(path)
        .await
        .read()
        .entries
        .iter()
        .map(|entry| entry.job.name())
        .collect()
}

fn temp_path(name: &str) -> (dirs::TempDir, File) {
    let dir = dirs::temp(name).unwrap();
    let path = dir.file("queue.json");
    (dir, path)
}

// ============================== GENERIC CASES ==================================== //

async fn check_empty_snapshot_loads_empty_queue<J: QueueJob>(name: &str) {
    let (_dir, path) = temp_path(name);

    let queue = Queue::<J>::from_snapshot(8, open::<J>(&path).await);

    assert!(queue.is_empty());
    assert_eq!(queue.len(), 0);
}

/// Persisted entries are loaded IN FULL even when they outnumber `capacity`:
/// capacity gates only new enqueues, so an over-capacity backlog drains before
/// the queue accepts more. Truncating the load would silently drop queued jobs,
/// since the next mutation rewrites the whole snapshot.
async fn check_from_snapshot_loads_an_over_capacity_backlog<J: QueueJob>(
    name: &str,
    make: fn(&str) -> J,
) {
    let (_dir, path) = temp_path(name);

    {
        let mut queue = Queue::<J>::from_snapshot(DEFAULT_CAPACITY, open::<J>(&path).await);
        for name in ["a.log", "b.log", "c.log"] {
            enqueue(&mut queue, make(name)).await;
        }
    }

    let mut queue = Queue::<J>::from_snapshot(2, open::<J>(&path).await);
    assert_eq!(queue.len(), 3);
    // the over-capacity backlog accepts nothing new until it drains
    assert!(queue.enqueue(make("d.log")).await.is_err());

    assert_eq!(
        drain(&mut queue).await,
        ["/data/a.log", "/data/b.log", "/data/c.log"]
    );
}

async fn check_enqueue_appends_in_fifo_order<J: QueueJob>(make: fn(&str) -> J) {
    let mut queue = Queue::<J>::new(DEFAULT_CAPACITY);
    assert!(queue.is_empty());
    for name in ["a.log", "b.log", "c.log"] {
        enqueue(&mut queue, make(name)).await;
    }
    assert!(!queue.is_empty());
    assert_eq!(queue.len(), 3);

    assert_eq!(
        drain(&mut queue).await,
        ["/data/a.log", "/data/b.log", "/data/c.log"]
    );
}

async fn check_enqueue_persists<J: QueueJob>(name: &str, make: fn(&str) -> J) {
    let (_dir, path) = temp_path(name);

    {
        let mut queue = Queue::<J>::from_snapshot(DEFAULT_CAPACITY, open::<J>(&path).await);
        enqueue(&mut queue, make("a.log")).await;
        enqueue(&mut queue, make("b.log")).await;
    }

    let reloaded = Queue::<J>::from_snapshot(DEFAULT_CAPACITY, open::<J>(&path).await);
    assert_eq!(reloaded.len(), 2);
    assert_eq!(on_disk::<J>(&path).await, ["/data/a.log", "/data/b.log"]);
}

/// Identity is per entry, not per job: two identical jobs are both queued,
/// under distinct ids. No field of `Job` is a usable key.
async fn check_duplicate_jobs_are_both_queued<J: QueueJob>(make: fn(&str) -> J) {
    let mut queue = Queue::<J>::new(DEFAULT_CAPACITY);
    let job = make("a.log");
    enqueue(&mut queue, job.clone()).await;
    enqueue(&mut queue, job).await;

    let first = queue.next_ready(now()).unwrap();
    queue.remove(first.id).await.unwrap();
    let second = queue.next_ready(now()).unwrap();
    assert_eq!(first.job, second.job);
    assert_ne!(first.id, second.id);
}

async fn check_new_entries_start_at_zero_attempts<J: QueueJob>(make: fn(&str) -> J) {
    let mut queue = Queue::<J>::new(DEFAULT_CAPACITY);
    enqueue(&mut queue, make("a.log")).await;

    let entry = queue.next_ready(now()).unwrap();
    assert_eq!(entry.attempts, 0);
    assert_eq!(entry.next_attempt_at, None);
}

/// A rejected enqueue leaves neither the queue nor the snapshot changed.
async fn check_rejection_does_not_persist<J: QueueJob>(name: &str, make: fn(&str) -> J) {
    let (_dir, path) = temp_path(name);

    {
        let mut queue = Queue::<J>::from_snapshot(1, open::<J>(&path).await);
        enqueue(&mut queue, make("a.log")).await;
        assert!(queue.enqueue(make("b.log")).await.is_err());
        assert_eq!(queue.len(), 1);
    }

    assert_eq!(on_disk::<J>(&path).await, ["/data/a.log"]);
}

/// Same-job is not a capacity bypass: a duplicate would grow the queue just as
/// a new job would, so it is rejected too.
async fn check_capacity_rejects_a_duplicate_job<J: QueueJob>(make: fn(&str) -> J) {
    let mut queue = Queue::<J>::new(1);
    enqueue(&mut queue, make("a.log")).await;

    assert!(queue.enqueue(make("a.log")).await.is_err());

    assert_eq!(drain(&mut queue).await, ["/data/a.log"]);
}

async fn check_full_queue_rejects_with_queue_full_err<J: QueueJob>(make: fn(&str) -> J) {
    let mut queue = Queue::<J>::new(1);
    enqueue(&mut queue, make("a.log")).await;

    let err = queue.enqueue(make("b.log")).await.unwrap_err();
    assert_eq!(err.label, J::LABEL);
    assert_eq!(err.capacity, 1);
    assert_eq!(err.name, "/data/b.log");
    assert_eq!(
        err.to_string(),
        format!(
            "{} queue is full (capacity 1); rejected job for file /data/b.log",
            J::LABEL
        )
    );
}

/// A persist that cannot write is logged and swallowed, never surfaced.
async fn check_persist_failure_is_swallowed<J: QueueJob>(name: &str, make: fn(&str) -> J) {
    let (_dir, path) = temp_path(name);
    let mut queue = Queue::<J>::from_snapshot(DEFAULT_CAPACITY, open::<J>(&path).await);

    // make the snapshot path unwritable: a DIRECTORY now sits there.
    files::delete(&path).await.unwrap();
    dirs::create(&Dir::new(path.path().clone())).await.unwrap();

    enqueue(&mut queue, make("a.log")).await;
    assert_eq!(queue.len(), 1);
}

/// `next_ready` returns a clone and deliberately leaves the entry on disk;
/// `remove` is the only point at which a job leaves durable storage.
async fn check_leaves_the_entry_on_disk_until_removed<J: QueueJob>(
    name: &str,
    make: fn(&str) -> J,
) {
    let (_dir, path) = temp_path(name);
    let mut queue = Queue::<J>::from_snapshot(DEFAULT_CAPACITY, open::<J>(&path).await);
    enqueue(&mut queue, make("a.log")).await;
    enqueue(&mut queue, make("b.log")).await;

    let entry = queue.next_ready(now()).unwrap();
    assert_eq!(on_disk::<J>(&path).await, ["/data/a.log", "/data/b.log"]);

    let removed = queue.remove(entry.id).await.unwrap();
    assert_eq!(removed.id, entry.id);
    assert_eq!(on_disk::<J>(&path).await, ["/data/b.log"]);
}

/// A persist landing between an entry's selection and its resolution must not
/// write the selected entry out of the snapshot. Stronger than
/// [`check_leaves_the_entry_on_disk_until_removed`], which only pins that
/// selection itself does not write: here an unrelated mutation does the
/// writing while the entry is in flight.
///
/// This is exercised at the `Queue` level because neither worker's run loop
/// can currently interleave a command with in-flight work — each awaits the
/// work to completion inside its match arm — so the interleaving is performed
/// directly on the queue. The test exists so that making a run loop
/// responsive to shutdown via `select!` cannot silently reintroduce the loss.
async fn check_persist_during_an_in_flight_entry_keeps_it_on_disk<J: QueueJob>(
    name: &str,
    make: fn(&str) -> J,
) {
    let (_dir, path) = temp_path(name);
    let mut queue = Queue::<J>::from_snapshot(DEFAULT_CAPACITY, open::<J>(&path).await);
    enqueue(&mut queue, make("a.log")).await;
    enqueue(&mut queue, make("b.log")).await;

    // the worker selects `a` and holds it: this is the in-flight state.
    let in_flight = queue.next_ready(now()).unwrap();
    assert_eq!(in_flight.job.name(), "/data/a.log");

    // an enqueue serviced mid-flight. Its persist is the write a `select!`
    // driven run loop would perform while `a` is still being worked.
    enqueue(&mut queue, make("c.log")).await;

    assert_eq!(
        on_disk::<J>(&path).await,
        ["/data/a.log", "/data/b.log", "/data/c.log"]
    );

    // `a` leaves disk only when the worker resolves it.
    queue.remove(in_flight.id).await.unwrap();
    assert_eq!(on_disk::<J>(&path).await, ["/data/b.log", "/data/c.log"]);
}

async fn check_next_ready_skips_waiting_entries<J: QueueJob>(make: fn(&str) -> J) {
    let mut queue = Queue::<J>::new(DEFAULT_CAPACITY);
    let deadline = now() + TimeDelta::hours(1);
    queue
        .requeue(QueueEntry {
            id: Uuid::new_v4(),
            job: make("waiting"),
            attempts: 1,
            next_attempt_at: Some(deadline),
        })
        .await;
    enqueue(&mut queue, make("ready_1")).await;
    enqueue(&mut queue, make("ready_2")).await;

    // skipping the ineligible head does not disturb FIFO among the remainder
    assert_eq!(drain(&mut queue).await, ["/data/ready_1", "/data/ready_2"]);

    assert!(queue.next_ready(now()).is_none());
    // the deadline itself is eligible: the comparison is inclusive
    assert_eq!(
        queue.next_ready(deadline).unwrap().job.name(),
        "/data/waiting"
    );
}

async fn check_next_ready_is_none_when_empty<J: QueueJob>() {
    let queue = Queue::<J>::new(DEFAULT_CAPACITY);
    assert!(queue.next_ready(now()).is_none());
}

async fn check_unknown_id_is_ignored<J: QueueJob>(make: fn(&str) -> J) {
    let mut queue = Queue::<J>::new(DEFAULT_CAPACITY);
    enqueue(&mut queue, make("a.log")).await;

    assert!(queue.remove(Uuid::new_v4()).await.is_none());
    assert_eq!(queue.len(), 1);
}

async fn check_removing_one_duplicate_leaves_the_other<J: QueueJob>(make: fn(&str) -> J) {
    let mut queue = Queue::<J>::new(DEFAULT_CAPACITY);
    let job = make("a.log");
    enqueue(&mut queue, job.clone()).await;
    enqueue(&mut queue, job).await;

    let entry = queue.next_ready(now()).unwrap();
    queue.remove(entry.id).await.unwrap();

    assert_eq!(queue.len(), 1);
    assert_ne!(queue.next_ready(now()).unwrap().id, entry.id);
}

/// A requeue is a rotation of an entry the queue already holds, not an
/// admission, so it is not capacity-gated and cannot grow the queue.
async fn check_requeue_rotates_to_the_tail<J: QueueJob>(make: fn(&str) -> J) {
    let mut queue = Queue::<J>::new(2);
    enqueue(&mut queue, make("a.log")).await;
    enqueue(&mut queue, make("b.log")).await;

    let entry = queue.next_ready(now()).unwrap();
    queue.requeue(entry).await;
    assert_eq!(queue.len(), 2);

    assert_eq!(drain(&mut queue).await, ["/data/b.log", "/data/a.log"]);
}

/// A requeue at capacity is admitted, and an entry the queue does not yet hold
/// is simply appended.
async fn check_requeue_at_capacity_admits_a_new_entry<J: QueueJob>(make: fn(&str) -> J) {
    let mut queue = Queue::<J>::new(1);
    enqueue(&mut queue, make("a.log")).await;

    queue
        .requeue(QueueEntry {
            id: Uuid::new_v4(),
            job: make("b.log"),
            attempts: 2,
            next_attempt_at: None,
        })
        .await;

    assert_eq!(queue.len(), 2);

    // the appended entry lands at the tail carrying its attempt count
    let head = queue.next_ready(now()).unwrap();
    assert_eq!(head.job.name(), "/data/a.log");
    queue.remove(head.id).await.unwrap();
    let tail = queue.next_ready(now()).unwrap();
    assert_eq!(tail.job.name(), "/data/b.log");
    assert_eq!(tail.attempts, 2);
}

/// A rotation is durable: the rotated order, and the fact that the rotated
/// entry is no longer at the head, both survive a restart.
async fn check_requeue_order_survives_a_reload<J: QueueJob>(name: &str, make: fn(&str) -> J) {
    let (_dir, path) = temp_path(name);
    let id;

    {
        let mut queue = Queue::<J>::from_snapshot(DEFAULT_CAPACITY, open::<J>(&path).await);
        enqueue(&mut queue, make("a.log")).await;
        enqueue(&mut queue, make("b.log")).await;
        let entry = queue.next_ready(now()).unwrap();
        id = entry.id;
        queue.requeue(entry).await;
    }

    let reloaded = Queue::<J>::from_snapshot(DEFAULT_CAPACITY, open::<J>(&path).await);
    assert_eq!(on_disk::<J>(&path).await, ["/data/b.log", "/data/a.log"]);
    assert_ne!(reloaded.next_ready(now()).unwrap().id, id);
}

/// The attempt counter and the backoff stamp ride along with the entry across
/// a requeue and a restart.
async fn check_attempts_and_deadline_survive_a_reload<J: QueueJob>(
    name: &str,
    make: fn(&str) -> J,
) {
    let (_dir, path) = temp_path(name);
    let deadline = now() + TimeDelta::hours(1);

    {
        let mut queue = Queue::<J>::from_snapshot(DEFAULT_CAPACITY, open::<J>(&path).await);
        enqueue(&mut queue, make("a.log")).await;
        let entry = queue.next_ready(now()).unwrap();
        queue
            .requeue(QueueEntry {
                attempts: 5,
                next_attempt_at: Some(deadline),
                ..entry
            })
            .await;
        assert_eq!(queue.len(), 1);
    }

    let reloaded = Queue::<J>::from_snapshot(DEFAULT_CAPACITY, open::<J>(&path).await);
    assert!(reloaded.next_ready(now()).is_none());
    let entry = reloaded.next_ready(deadline).unwrap();
    assert_eq!(entry.attempts, 5);
    assert_eq!(entry.next_attempt_at, Some(deadline));
}

async fn check_count_ready<J: QueueJob>(make: fn(&str) -> J) {
    let mut queue = Queue::<J>::new(DEFAULT_CAPACITY);
    assert_eq!(queue.count_ready(now()), 0);

    enqueue(&mut queue, make("a.log")).await;
    enqueue(&mut queue, make("b.log")).await;
    queue
        .requeue(QueueEntry {
            id: Uuid::new_v4(),
            job: make("waiting"),
            attempts: 1,
            next_attempt_at: Some(now() + TimeDelta::hours(1)),
        })
        .await;

    assert_eq!(queue.count_ready(now()), 2);
    assert_eq!(queue.len(), 3);
}

async fn check_reset_invalid_deadlines<J: QueueJob>(make: fn(&str) -> J) {
    let mut queue = Queue::<J>::new(DEFAULT_CAPACITY);
    let horizon = now() + TimeDelta::hours(24);
    let inside = horizon - TimeDelta::seconds(1);
    let beyond = horizon + TimeDelta::seconds(1);
    for (name, deadline) in [("a.log", inside), ("b.log", horizon), ("c.log", beyond)] {
        queue
            .requeue(QueueEntry {
                id: Uuid::new_v4(),
                job: make(name),
                attempts: 1,
                next_attempt_at: Some(deadline),
            })
            .await;
    }

    queue.reset_invalid_deadlines(horizon).await;

    let mut drained = Vec::new();
    while let Some(entry) = queue.next_ready(beyond) {
        drained.push((entry.job.name(), entry.next_attempt_at));
        queue.remove(entry.id).await;
    }
    assert_eq!(
        drained,
        vec![
            ("/data/a.log".to_string(), Some(inside)),
            ("/data/b.log".to_string(), Some(horizon)),
            ("/data/c.log".to_string(), Some(horizon)),
        ]
    );
}

/// Nothing beyond the horizon means nothing moves — the early-return path.
async fn check_reset_invalid_deadlines_is_a_noop_when_all_inside<J: QueueJob>(make: fn(&str) -> J) {
    let mut queue = Queue::<J>::new(DEFAULT_CAPACITY);
    let horizon = now() + TimeDelta::hours(24);
    enqueue(&mut queue, make("a.log")).await;

    queue.reset_invalid_deadlines(horizon).await;

    assert_eq!(queue.next_ready(now()).unwrap().next_attempt_at, None);
}

/// A reset that actually moves a deadline is durable: the pulled-back stamp
/// survives a restart, without waiting for a later mutation to write it.
async fn check_reset_invalid_deadlines_persists<J: QueueJob>(name: &str, make: fn(&str) -> J) {
    let (_dir, path) = temp_path(name);
    let horizon = now() + TimeDelta::hours(24);
    let beyond = horizon + TimeDelta::seconds(1);

    {
        let mut queue = Queue::<J>::from_snapshot(DEFAULT_CAPACITY, open::<J>(&path).await);
        queue
            .requeue(QueueEntry {
                id: Uuid::new_v4(),
                job: make("a.log"),
                attempts: 1,
                next_attempt_at: Some(beyond),
            })
            .await;
        queue.reset_invalid_deadlines(horizon).await;
    }

    let reloaded = Queue::<J>::from_snapshot(DEFAULT_CAPACITY, open::<J>(&path).await);
    let entry = reloaded.next_ready(horizon).unwrap();
    assert_eq!(entry.next_attempt_at, Some(horizon));
}

async fn check_earliest_next_attempt<J: QueueJob>(make: fn(&str) -> J) {
    let mut queue = Queue::<J>::new(DEFAULT_CAPACITY);
    assert_eq!(queue.earliest_next_attempt(), None);

    let t1 = now() + TimeDelta::hours(1);
    let t2 = now() + TimeDelta::hours(2);
    for (name, deadline) in [("a.log", t2), ("b.log", t1)] {
        queue
            .requeue(QueueEntry {
                id: Uuid::new_v4(),
                job: make(name),
                attempts: 1,
                next_attempt_at: Some(deadline),
            })
            .await;
    }
    assert_eq!(queue.earliest_next_attempt(), Some(t1));

    // an entry with no deadline counts as MIN_UTC
    enqueue(&mut queue, make("c.log")).await;
    assert_eq!(
        queue.earliest_next_attempt(),
        Some(DateTime::<Utc>::MIN_UTC)
    );
}

/// An entry written before a field existed still loads, rather than failing to
/// deserialize — which `new_with_default` would turn into a silent wipe.
async fn check_missing_optional_fields_default<J: QueueJob>(name: &str, make: fn(&str) -> J) {
    let (_dir, path) = temp_path(name);
    let snapshot = QueueSnapshot {
        entries: vec![QueueEntry {
            id: Uuid::new_v4(),
            job: make("a.log"),
            attempts: 7,
            next_attempt_at: Some(now()),
        }],
    };
    let mut value = serde_json::to_value(&snapshot).unwrap();
    for entry in value["entries"].as_array_mut().unwrap() {
        let entry = entry.as_object_mut().unwrap();
        entry.remove("id");
        entry.remove("attempts");
        entry.remove("next_attempt_at");
    }
    files::write_string(&path, &value.to_string(), WriteOptions::OVERWRITE_ATOMIC)
        .await
        .unwrap();

    let mut queue = Queue::<J>::from_snapshot(8, open::<J>(&path).await);
    assert_eq!(queue.len(), 1);
    let entry = queue.next_ready(now()).unwrap();
    assert_eq!(entry.attempts, 0);
    assert_eq!(entry.next_attempt_at, None);
    // the minted id is a working handle for removal
    assert!(queue.remove(entry.id).await.is_some());
    assert!(queue.is_empty());
}

// ============================== UPLOAD JOBS ====================================== //

mod upload_jobs {
    use super::*;

    #[tokio::test]
    async fn empty_snapshot_loads_empty_queue() {
        check_empty_snapshot_loads_empty_queue::<upload::Job>("upload-generic-queue-test").await;
    }

    #[tokio::test]
    async fn from_snapshot_loads_an_over_capacity_backlog() {
        check_from_snapshot_loads_an_over_capacity_backlog("upload-generic-queue-test", upload_job)
            .await;
    }

    #[tokio::test]
    async fn enqueue_appends_in_fifo_order() {
        check_enqueue_appends_in_fifo_order(upload_job).await;
    }

    #[tokio::test]
    async fn enqueue_persists() {
        check_enqueue_persists("upload-generic-queue-test", upload_job).await;
    }

    #[tokio::test]
    async fn duplicate_jobs_are_both_queued() {
        check_duplicate_jobs_are_both_queued(upload_job).await;
    }

    #[tokio::test]
    async fn new_entries_start_at_zero_attempts() {
        check_new_entries_start_at_zero_attempts(upload_job).await;
    }

    #[tokio::test]
    async fn rejection_does_not_persist() {
        check_rejection_does_not_persist("upload-generic-queue-test", upload_job).await;
    }

    #[tokio::test]
    async fn capacity_rejects_a_duplicate_job() {
        check_capacity_rejects_a_duplicate_job(upload_job).await;
    }

    #[tokio::test]
    async fn persist_failure_is_swallowed() {
        check_persist_failure_is_swallowed("upload-generic-queue-test", upload_job).await;
    }

    #[tokio::test]
    async fn leaves_the_entry_on_disk_until_removed() {
        check_leaves_the_entry_on_disk_until_removed("upload-generic-queue-test", upload_job).await;
    }

    #[tokio::test]
    async fn persist_during_an_in_flight_entry_keeps_it_on_disk() {
        check_persist_during_an_in_flight_entry_keeps_it_on_disk(
            "upload-generic-queue-test",
            upload_job,
        )
        .await;
    }

    #[tokio::test]
    async fn next_ready_skips_waiting_entries() {
        check_next_ready_skips_waiting_entries(upload_job).await;
    }

    #[tokio::test]
    async fn next_ready_is_none_when_empty() {
        check_next_ready_is_none_when_empty::<upload::Job>().await;
    }

    #[tokio::test]
    async fn unknown_id_is_ignored() {
        check_unknown_id_is_ignored(upload_job).await;
    }

    #[tokio::test]
    async fn removing_one_duplicate_leaves_the_other() {
        check_removing_one_duplicate_leaves_the_other(upload_job).await;
    }

    #[tokio::test]
    async fn requeue_rotates_to_the_tail() {
        check_requeue_rotates_to_the_tail(upload_job).await;
    }

    #[tokio::test]
    async fn requeue_at_capacity_admits_a_new_entry() {
        check_requeue_at_capacity_admits_a_new_entry(upload_job).await;
    }

    #[tokio::test]
    async fn requeue_order_survives_a_reload() {
        check_requeue_order_survives_a_reload("upload-generic-queue-test", upload_job).await;
    }

    #[tokio::test]
    async fn attempts_and_deadline_survive_a_reload() {
        check_attempts_and_deadline_survive_a_reload("upload-generic-queue-test", upload_job).await;
    }

    #[tokio::test]
    async fn count_ready_counts_only_ready_entries() {
        check_count_ready(upload_job).await;
    }

    #[tokio::test]
    async fn reset_invalid_deadlines_pulls_back_only_beyond_the_horizon() {
        check_reset_invalid_deadlines(upload_job).await;
    }

    #[tokio::test]
    async fn reset_invalid_deadlines_is_a_noop_when_all_inside() {
        check_reset_invalid_deadlines_is_a_noop_when_all_inside(upload_job).await;
    }

    #[tokio::test]
    async fn reset_invalid_deadlines_persists() {
        check_reset_invalid_deadlines_persists("upload-generic-queue-test", upload_job).await;
    }

    #[tokio::test]
    async fn earliest_next_attempt_returns_the_minimum() {
        check_earliest_next_attempt(upload_job).await;
    }

    #[tokio::test]
    async fn missing_optional_fields_default() {
        check_missing_optional_fields_default("upload-generic-queue-test", upload_job).await;
    }

    /// An upload job is due the moment it arrives: `due_at` is `MIN_UTC`, so
    /// readiness is decided by `next_attempt_at` alone.
    #[test]
    fn due_at_is_min_utc() {
        assert_eq!(
            QueueJob::due_at(&upload_job("a.log")),
            DateTime::<Utc>::MIN_UTC
        );
    }

    #[tokio::test]
    async fn full_queue_rejects_with_queue_full_err() {
        check_full_queue_rejects_with_queue_full_err(upload_job).await;
    }
}

// ============================== RETENTION JOBS =================================== //

mod retention_jobs {
    use super::*;

    #[tokio::test]
    async fn empty_snapshot_loads_empty_queue() {
        check_empty_snapshot_loads_empty_queue::<retention::Job>("delete-generic-queue-test").await;
    }

    #[tokio::test]
    async fn from_snapshot_loads_an_over_capacity_backlog() {
        check_from_snapshot_loads_an_over_capacity_backlog(
            "delete-generic-queue-test",
            retention_job,
        )
        .await;
    }

    #[tokio::test]
    async fn enqueue_appends_in_fifo_order() {
        check_enqueue_appends_in_fifo_order(retention_job).await;
    }

    #[tokio::test]
    async fn enqueue_persists() {
        check_enqueue_persists("delete-generic-queue-test", retention_job).await;
    }

    #[tokio::test]
    async fn duplicate_jobs_are_both_queued() {
        check_duplicate_jobs_are_both_queued(retention_job).await;
    }

    #[tokio::test]
    async fn new_entries_start_at_zero_attempts() {
        check_new_entries_start_at_zero_attempts(retention_job).await;
    }

    #[tokio::test]
    async fn rejection_does_not_persist() {
        check_rejection_does_not_persist("delete-generic-queue-test", retention_job).await;
    }

    #[tokio::test]
    async fn capacity_rejects_a_duplicate_job() {
        check_capacity_rejects_a_duplicate_job(retention_job).await;
    }

    #[tokio::test]
    async fn persist_failure_is_swallowed() {
        check_persist_failure_is_swallowed("delete-generic-queue-test", retention_job).await;
    }

    #[tokio::test]
    async fn leaves_the_entry_on_disk_until_removed() {
        check_leaves_the_entry_on_disk_until_removed("delete-generic-queue-test", retention_job)
            .await;
    }

    #[tokio::test]
    async fn persist_during_an_in_flight_entry_keeps_it_on_disk() {
        check_persist_during_an_in_flight_entry_keeps_it_on_disk(
            "delete-generic-queue-test",
            retention_job,
        )
        .await;
    }

    #[tokio::test]
    async fn next_ready_skips_waiting_entries() {
        check_next_ready_skips_waiting_entries(retention_job).await;
    }

    #[tokio::test]
    async fn next_ready_is_none_when_empty() {
        check_next_ready_is_none_when_empty::<retention::Job>().await;
    }

    #[tokio::test]
    async fn unknown_id_is_ignored() {
        check_unknown_id_is_ignored(retention_job).await;
    }

    #[tokio::test]
    async fn removing_one_duplicate_leaves_the_other() {
        check_removing_one_duplicate_leaves_the_other(retention_job).await;
    }

    #[tokio::test]
    async fn requeue_rotates_to_the_tail() {
        check_requeue_rotates_to_the_tail(retention_job).await;
    }

    #[tokio::test]
    async fn requeue_at_capacity_admits_a_new_entry() {
        check_requeue_at_capacity_admits_a_new_entry(retention_job).await;
    }

    #[tokio::test]
    async fn requeue_order_survives_a_reload() {
        check_requeue_order_survives_a_reload("delete-generic-queue-test", retention_job).await;
    }

    #[tokio::test]
    async fn attempts_and_deadline_survive_a_reload() {
        check_attempts_and_deadline_survive_a_reload("delete-generic-queue-test", retention_job)
            .await;
    }

    #[tokio::test]
    async fn count_ready_counts_only_ready_entries() {
        check_count_ready(retention_job).await;
    }

    #[tokio::test]
    async fn reset_invalid_deadlines_pulls_back_only_beyond_the_horizon() {
        check_reset_invalid_deadlines(retention_job).await;
    }

    #[tokio::test]
    async fn reset_invalid_deadlines_is_a_noop_when_all_inside() {
        check_reset_invalid_deadlines_is_a_noop_when_all_inside(retention_job).await;
    }

    #[tokio::test]
    async fn reset_invalid_deadlines_persists() {
        check_reset_invalid_deadlines_persists("delete-generic-queue-test", retention_job).await;
    }

    #[tokio::test]
    async fn earliest_next_attempt_returns_the_minimum() {
        check_earliest_next_attempt(retention_job).await;
    }

    #[tokio::test]
    async fn missing_optional_fields_default() {
        check_missing_optional_fields_default("delete-generic-queue-test", retention_job).await;
    }

    /// The other state of the `due_at` hook: a retention job is not ready until
    /// its TTL has elapsed, and `next_ready` / `count_ready` both honour it.
    #[tokio::test]
    async fn a_job_whose_ttl_has_not_elapsed_is_not_ready() {
        let mut queue = Queue::<retention::Job>::new(DEFAULT_CAPACITY);
        enqueue(&mut queue, retention_job_with_ttl("waiting", 5000)).await;
        enqueue(&mut queue, retention_job_with_ttl("due", 500)).await;

        // due_at is inclusive: "due" is due at exactly `now`
        assert_eq!(queue.count_ready(now()), 1);
        assert_eq!(queue.next_ready(now()).unwrap().job.name(), "/data/due");

        // the waiting job becomes ready once its TTL elapses
        assert_eq!(queue.count_ready(now() + TimeDelta::hours(2)), 2);
    }

    #[tokio::test]
    async fn full_queue_rejects_with_queue_full_err() {
        check_full_queue_rejects_with_queue_full_err(retention_job).await;
    }
}
