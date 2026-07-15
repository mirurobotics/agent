//! The generic job queue, written once and instantiated per worker.
//!
//! Every case is written once as a generic function in [`cases`] over
//! `J: QueueJob`, grouped into a submodule named for the queue method it
//! exercises. Each worker's own test file (`upload/queue.rs`,
//! `retention/queue.rs`) invokes [`queue_suite!`] with its job factory, which
//! emits one `mod` per group holding one `#[tokio::test]` per case, each named
//! exactly after the case function — so a failing test path
//! (`upload::queue::enqueue::persists`) greps straight to its body here. The
//! suite therefore runs against `upload::Job` (always due on arrival) and
//! `retention::Job` (due only once its TTL elapses), proving the `due_at` hook
//! in both states.
//!
//! This file itself contains no tests: only the fixtures, the generic cases,
//! and the macro.

// internal crates
use miru_agent::data_uploads::queue::{
    Queue, QueueEntry, QueueJob, QueueSnapshot, QueueSnapshotFile,
};
use miru_agent::filesys::state_file::Options;
use miru_agent::filesys::{dirs, files, Dir, File, PathExt, WriteOptions};

// external crates
use chrono::{DateTime, TimeDelta, Utc};
use uuid::Uuid;

pub const DEFAULT_CAPACITY: usize = 4096;

/// The instant every test selects at. Both job factories produce jobs that are
/// due at it.
pub fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(2000, 0).unwrap()
}

/// A fresh snapshot handle over `path`. Reopening the same path returns a
/// handle whose in-memory cache reflects what was previously persisted.
async fn open<J: QueueJob>(path: &File) -> QueueSnapshotFile<J> {
    QueueSnapshotFile::<J>::open(
        path.clone(),
        Options {
            default: Some(QueueSnapshot::<J>::default()),
            ..Default::default()
        },
    )
    .await
    .unwrap()
}

/// `enqueue` without requiring the error to be `Debug`.
pub async fn enqueue<J: QueueJob>(queue: &mut Queue<J>, job: J) {
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

/// The case bodies, grouped by the queue method under test. Each case is named
/// exactly what its generated `#[tokio::test]` is called, and takes one of two
/// uniform signatures:
///
/// - disk-backed: `(tmp: &str, make: fn(&str) -> J)` — `tmp` names the temp dir
/// - in-memory: `(make: fn(&str) -> J)`
///
/// [`super::queue_suite`] lists each group's two sublists and emits a matching
/// `mod` of wrappers.
pub mod cases {
    use super::*;

    // ----------------------------- from_snapshot --------------------------------- //

    pub mod from_snapshot {
        use super::*;

        pub async fn empty_loads_empty_queue<J: QueueJob>(tmp: &str, _make: fn(&str) -> J) {
            let (_dir, path) = temp_path(tmp);

            let queue = Queue::<J>::from_snapshot(8, open::<J>(&path).await);

            assert!(queue.is_empty());
            assert_eq!(queue.len(), 0);
        }

        pub async fn loads_an_over_capacity_backlog<J: QueueJob>(tmp: &str, make: fn(&str) -> J) {
            let (_dir, path) = temp_path(tmp);

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

        pub async fn missing_optional_fields_default<J: QueueJob>(tmp: &str, make: fn(&str) -> J) {
            let (_dir, path) = temp_path(tmp);
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
    }

    // -------------------------------- enqueue ------------------------------------ //

    pub mod enqueue {
        use super::*;

        pub async fn appends_in_fifo_order<J: QueueJob>(make: fn(&str) -> J) {
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

        pub async fn persists<J: QueueJob>(tmp: &str, make: fn(&str) -> J) {
            let (_dir, path) = temp_path(tmp);

            {
                let mut queue = Queue::<J>::from_snapshot(DEFAULT_CAPACITY, open::<J>(&path).await);
                enqueue(&mut queue, make("a.log")).await;
                enqueue(&mut queue, make("b.log")).await;
            }

            let reloaded = Queue::<J>::from_snapshot(DEFAULT_CAPACITY, open::<J>(&path).await);
            assert_eq!(reloaded.len(), 2);
            assert_eq!(on_disk::<J>(&path).await, ["/data/a.log", "/data/b.log"]);
        }

        pub async fn duplicate_jobs_are_both_queued<J: QueueJob>(make: fn(&str) -> J) {
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

        pub async fn new_entries_start_at_zero_attempts<J: QueueJob>(make: fn(&str) -> J) {
            let mut queue = Queue::<J>::new(DEFAULT_CAPACITY);
            enqueue(&mut queue, make("a.log")).await;

            let entry = queue.next_ready(now()).unwrap();
            assert_eq!(entry.attempts, 0);
            assert_eq!(entry.next_attempt_at, None);
        }

        pub async fn capacity_rejects_a_duplicate_job<J: QueueJob>(make: fn(&str) -> J) {
            let mut queue = Queue::<J>::new(1);
            enqueue(&mut queue, make("a.log")).await;

            assert!(queue.enqueue(make("a.log")).await.is_err());

            assert_eq!(drain(&mut queue).await, ["/data/a.log"]);
        }

        pub async fn full_queue_rejects_with_queue_full_err<J: QueueJob>(make: fn(&str) -> J) {
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

        pub async fn rejection_does_not_persist<J: QueueJob>(tmp: &str, make: fn(&str) -> J) {
            let (_dir, path) = temp_path(tmp);

            {
                let mut queue = Queue::<J>::from_snapshot(1, open::<J>(&path).await);
                enqueue(&mut queue, make("a.log")).await;
                assert!(queue.enqueue(make("b.log")).await.is_err());
                assert_eq!(queue.len(), 1);
            }

            assert_eq!(on_disk::<J>(&path).await, ["/data/a.log"]);
        }

        pub async fn persist_failure_is_swallowed<J: QueueJob>(tmp: &str, make: fn(&str) -> J) {
            let (_dir, path) = temp_path(tmp);
            let mut queue = Queue::<J>::from_snapshot(DEFAULT_CAPACITY, open::<J>(&path).await);

            // make the snapshot path unwritable: a DIRECTORY now sits there.
            files::delete(&path).await.unwrap();
            dirs::create(&Dir::new(path.path().clone())).await.unwrap();

            enqueue(&mut queue, make("a.log")).await;
            assert_eq!(queue.len(), 1);
        }
    }

    // ------------------------------- next_ready ---------------------------------- //

    /// Selection: which entry is eligible, and what selection does *not* do.
    pub mod next_ready {
        use super::*;

        pub async fn skips_waiting_entries<J: QueueJob>(make: fn(&str) -> J) {
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

        pub async fn is_none_when_empty<J: QueueJob>(_make: fn(&str) -> J) {
            let queue = Queue::<J>::new(DEFAULT_CAPACITY);
            assert!(queue.next_ready(now()).is_none());
        }

        pub async fn leaves_the_entry_on_disk_until_removed<J: QueueJob>(
            tmp: &str,
            make: fn(&str) -> J,
        ) {
            let (_dir, path) = temp_path(tmp);
            let mut queue = Queue::<J>::from_snapshot(DEFAULT_CAPACITY, open::<J>(&path).await);
            enqueue(&mut queue, make("a.log")).await;
            enqueue(&mut queue, make("b.log")).await;

            let entry = queue.next_ready(now()).unwrap();
            assert_eq!(on_disk::<J>(&path).await, ["/data/a.log", "/data/b.log"]);

            let removed = queue.remove(entry.id).await.unwrap();
            assert_eq!(removed.id, entry.id);
            assert_eq!(on_disk::<J>(&path).await, ["/data/b.log"]);
        }
    }

    // --------------------------------- remove ------------------------------------ //

    pub mod remove {
        use super::*;

        pub async fn unknown_id_is_ignored<J: QueueJob>(make: fn(&str) -> J) {
            let mut queue = Queue::<J>::new(DEFAULT_CAPACITY);
            enqueue(&mut queue, make("a.log")).await;

            assert!(queue.remove(Uuid::new_v4()).await.is_none());
            assert_eq!(queue.len(), 1);
        }

        pub async fn one_duplicate_leaves_the_other<J: QueueJob>(make: fn(&str) -> J) {
            let mut queue = Queue::<J>::new(DEFAULT_CAPACITY);
            let job = make("a.log");
            enqueue(&mut queue, job.clone()).await;
            enqueue(&mut queue, job).await;

            let entry = queue.next_ready(now()).unwrap();
            queue.remove(entry.id).await.unwrap();

            assert_eq!(queue.len(), 1);
            assert_ne!(queue.next_ready(now()).unwrap().id, entry.id);
        }
    }

    // -------------------------------- requeue ------------------------------------ //

    pub mod requeue {
        use super::*;

        pub async fn rotates_to_the_tail<J: QueueJob>(make: fn(&str) -> J) {
            let mut queue = Queue::<J>::new(2);
            enqueue(&mut queue, make("a.log")).await;
            enqueue(&mut queue, make("b.log")).await;

            let entry = queue.next_ready(now()).unwrap();
            queue.requeue(entry).await;
            assert_eq!(queue.len(), 2);

            assert_eq!(drain(&mut queue).await, ["/data/b.log", "/data/a.log"]);
        }

        pub async fn at_capacity_admits_a_new_entry<J: QueueJob>(make: fn(&str) -> J) {
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

        pub async fn order_survives_a_reload<J: QueueJob>(tmp: &str, make: fn(&str) -> J) {
            let (_dir, path) = temp_path(tmp);
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

        pub async fn attempts_and_deadline_survive_a_reload<J: QueueJob>(
            tmp: &str,
            make: fn(&str) -> J,
        ) {
            let (_dir, path) = temp_path(tmp);
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
    }

    // ------------------------------- count_ready --------------------------------- //

    pub mod count_ready {
        use super::*;

        pub async fn counts_only_ready_entries<J: QueueJob>(make: fn(&str) -> J) {
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
    }

    // -------------------------- reset_invalid_deadlines -------------------------- //

    pub mod reset_invalid_deadlines {
        use super::*;

        pub async fn pulls_back_only_beyond_the_horizon<J: QueueJob>(make: fn(&str) -> J) {
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

        pub async fn is_a_noop_when_all_inside<J: QueueJob>(make: fn(&str) -> J) {
            let mut queue = Queue::<J>::new(DEFAULT_CAPACITY);
            let horizon = now() + TimeDelta::hours(24);
            enqueue(&mut queue, make("a.log")).await;

            queue.reset_invalid_deadlines(horizon).await;

            assert_eq!(queue.next_ready(now()).unwrap().next_attempt_at, None);
        }

        pub async fn persists<J: QueueJob>(tmp: &str, make: fn(&str) -> J) {
            let (_dir, path) = temp_path(tmp);
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
    }

    // --------------------------- earliest_next_attempt --------------------------- //

    pub mod earliest_next_attempt {
        use super::*;

        pub async fn returns_the_minimum<J: QueueJob>(make: fn(&str) -> J) {
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
    }

    // -------------------------------- durability --------------------------------- //

    /// What the snapshot must still hold while a worker is mid-job.
    pub mod durability {
        use super::*;

        /// A persist landing between an entry's selection and its resolution
        /// must not write the selected entry out of the snapshot. Stronger than
        /// [`super::next_ready::leaves_the_entry_on_disk_until_removed`], which
        /// only pins that selection itself does not write: here an unrelated
        /// mutation does the writing while the entry is in flight.
        ///
        /// This is exercised at the `Queue` level because neither worker's run
        /// loop can currently interleave a command with in-flight work — each
        /// awaits the work to completion inside its match arm — so the
        /// interleaving is performed directly on the queue. The test exists so
        /// that making a run loop responsive to shutdown via `select!` cannot
        /// silently reintroduce the loss.
        pub async fn persist_during_an_in_flight_entry_keeps_it_on_disk<J: QueueJob>(
            tmp: &str,
            make: fn(&str) -> J,
        ) {
            let (_dir, path) = temp_path(tmp);
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
    }
}

// ============================== THE SUITE MACRO ================================== //

/// Emits one `mod` per named group, holding one `#[tokio::test]` per named
/// case. Within a group, `disk` cases are called with `($tmp, $make)` and `mem`
/// cases with `($make)`; either list may be empty. Each generated test is named
/// exactly after the case it calls, under a module named after its group.
macro_rules! queue_suite_emit {
    (
        $make:expr, $tmp:literal,
        $(
            $group:ident {
                disk: [$($disk:ident),* $(,)?],
                mem: [$($mem:ident),* $(,)?] $(,)?
            }
        ),* $(,)?
    ) => {
        $(
            mod $group {
                use super::*;

                $(
                    #[tokio::test]
                    async fn $disk() {
                        $crate::data_uploads::queue::cases::$group::$disk($tmp, $make).await;
                    }
                )*
                $(
                    #[tokio::test]
                    async fn $mem() {
                        $crate::data_uploads::queue::cases::$group::$mem($make).await;
                    }
                )*
            }
        )*
    };
}
pub(crate) use queue_suite_emit;

/// Instantiates the whole generic queue suite for one job type.
///
/// `$make` is the job factory (`fn(&str) -> J`) and `$tmp` is the temp-dir name
/// the disk-backed cases allocate under. Invoked once per worker, from that
/// worker's own test file.
macro_rules! queue_suite {
    ($make:expr, $tmp:literal) => {
        $crate::data_uploads::queue::queue_suite_emit! {
            $make, $tmp,
            from_snapshot {
                disk: [
                    empty_loads_empty_queue,
                    loads_an_over_capacity_backlog,
                    missing_optional_fields_default,
                ],
                mem: [],
            },
            enqueue {
                disk: [
                    persists,
                    rejection_does_not_persist,
                    persist_failure_is_swallowed,
                ],
                mem: [
                    appends_in_fifo_order,
                    duplicate_jobs_are_both_queued,
                    new_entries_start_at_zero_attempts,
                    capacity_rejects_a_duplicate_job,
                    full_queue_rejects_with_queue_full_err,
                ],
            },
            next_ready {
                disk: [leaves_the_entry_on_disk_until_removed],
                mem: [skips_waiting_entries, is_none_when_empty],
            },
            remove {
                disk: [],
                mem: [unknown_id_is_ignored, one_duplicate_leaves_the_other],
            },
            requeue {
                disk: [order_survives_a_reload, attempts_and_deadline_survive_a_reload],
                mem: [rotates_to_the_tail, at_capacity_admits_a_new_entry],
            },
            count_ready {
                disk: [],
                mem: [counts_only_ready_entries],
            },
            reset_invalid_deadlines {
                disk: [persists],
                mem: [
                    pulls_back_only_beyond_the_horizon,
                    is_a_noop_when_all_inside,
                ],
            },
            earliest_next_attempt {
                disk: [],
                mem: [returns_the_minimum],
            },
            durability {
                disk: [persist_during_an_in_flight_entry_keeps_it_on_disk],
                mem: [],
            },
        }
    };
}
pub(crate) use queue_suite;
