// standard crates
use std::collections::VecDeque;

// internal crates
use crate::data_uploads::upload::job::Job;
use crate::filesys::state_file::SingleThreadStateFile;
use crate::models::Patch;

// external crates
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueueEntry {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    pub job: Job,
    pub attempts: u32,
    /// Earliest instant this entry is eligible to be popped; `None` means
    /// "eligible now".
    #[serde(default)]
    pub next_attempt_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct QueueSnapshot {
    pub entries: Vec<QueueEntry>,
}

impl Patch<QueueSnapshot> for QueueSnapshot {
    fn patch(&mut self, patch: QueueSnapshot) {
        *self = patch;
    }
}

/// The queue's persistence handle: an atomically-written JSON snapshot with an
/// in-memory cache. Mirrors the scanner's `ScanSnapshotFile`.
pub type QueueSnapshotFile = SingleThreadStateFile<QueueSnapshot, QueueSnapshot>;

pub struct Queue {
    jobs: VecDeque<QueueEntry>,
    capacity: usize,
    snapshot_file: Option<QueueSnapshotFile>,
}

impl Queue {
    /// An in-memory-only queue with no disk persistence.
    pub fn new(capacity: usize) -> Self {
        Self {
            jobs: VecDeque::new(),
            capacity,
            snapshot_file: None,
        }
    }

    /// A disk-backed queue: `snapshot_file`'s persisted entries seed the initial
    /// backlog, and every subsequent mutation is written back. Persisted entries are
    /// loaded in full even if they exceed `capacity` (e.g. after `capacity` was
    /// lowered). Each enqueue over capacity evicts exactly one entry before
    /// admitting the new job, so an over-capacity backlog never grows past the
    /// length it was loaded with — it shrinks one entry per enqueue and drains
    /// through uploads.
    pub fn from_snapshot(capacity: usize, snapshot_file: QueueSnapshotFile) -> Self {
        let jobs = snapshot_file.read().entries.iter().cloned().collect();
        Self {
            jobs,
            capacity,
            snapshot_file: Some(snapshot_file),
        }
    }

    pub fn len(&self) -> usize {
        self.jobs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    /// Push a new job at the tail. A new job is never refused: when the queue
    /// is at capacity, exactly one existing entry is evicted (and logged at
    /// `error!`) to make room. The victim is the entry with the highest
    /// `attempts`, ties broken by the oldest `job.first_observed_at`.
    ///
    /// Note that during a network outage every entry has `attempts == 0` —
    /// network-classified failures are exempt from the attempt budget — so the
    /// policy degenerates to its tie-break and the real behavior is "keep the
    /// newest files, drop the oldest".
    ///
    /// `now` is used only to report the evicted entry's age.
    pub async fn enqueue(&mut self, job: Job, now: DateTime<Utc>) {
        self.make_room(now);
        self.jobs.push_back(QueueEntry {
            id: Uuid::new_v4(),
            job,
            attempts: 0,
            next_attempt_at: None,
        });
        self.persist().await;
        info!("upload: job enqueued; queue length {}", self.jobs.len());
    }

    /// Drop the entry with `id` and persist the shortened queue. This is the
    /// only point at which a job leaves durable storage, so call it only once
    /// the backend has confirmed the upload or the job has been explicitly
    /// dropped. Unknown ids are ignored.
    pub async fn remove(&mut self, id: Uuid) -> Option<QueueEntry> {
        if let Some(entry) = self.remove_impl(id) {
            self.persist().await;
            info!("upload: job removed; queue length {}", self.jobs.len());
            return Some(entry);
        }
        None
    }

    fn remove_impl(&mut self, id: Uuid) -> Option<QueueEntry> {
        let idx = self.jobs.iter().position(|entry| entry.id == id)?;
        self.jobs.remove(idx)
    }

    /// Move an entry the queue still holds to the tail, replacing what is
    /// queued under its id so the caller's updated attempt count and backoff
    /// deadline are persisted. Returns `true` when the entry was moved.
    ///
    /// Returns `false` — a no-op, with nothing pushed and nothing persisted —
    /// when the queue no longer holds `entry.id`, because it was confirmed,
    /// dropped, or evicted while the caller held its clone. [`Self::enqueue`]
    /// is therefore the only path that grows the queue, which is what keeps
    /// capacity enforcement in one place and stops an evicted in-flight entry
    /// from being resurrected by its requeue.
    pub async fn requeue(&mut self, entry: QueueEntry) -> bool {
        let Some(idx) = self.jobs.iter().position(|queued| queued.id == entry.id) else {
            return false;
        };
        self.jobs.remove(idx);
        self.jobs.push_back(entry);
        self.persist().await;
        info!("upload: job requeued; queue length {}", self.jobs.len());
        true
    }

    /// Return a clone of the first entry whose `next_attempt_at` is `None` or
    /// `<= now`. The entry is deliberately LEFT in the queue — and so in the
    /// persisted snapshot — until the caller passes its id back to
    /// [`Self::remove`] or hands the entry to [`Self::requeue`]. That is what
    /// makes the queue at-least-once: an upload cut short by a crash, a kill,
    /// or a shutdown is still on disk at the next boot and is retried.
    /// Returns `None` when no entry is eligible.
    pub fn next_ready(&self, now: DateTime<Utc>) -> Option<QueueEntry> {
        let entry = self
            .jobs
            .iter()
            .find(|entry| entry.next_attempt_at.is_none_or(|at| at <= now))?;
        info!("upload: job dequeued; queue length {}", self.jobs.len());
        Some(entry.clone())
    }

    /// Pull every `next_attempt_at` strictly beyond `horizon` back to it, so no
    /// entry can be deferred past a deadline the caller considers reachable.
    pub fn reset_invalid_deadlines(&mut self, horizon: DateTime<Utc>) {
        let mut reset = 0;
        for entry in self.jobs.iter_mut() {
            if entry.next_attempt_at.is_some_and(|at| at > horizon) {
                entry.next_attempt_at = Some(horizon);
                reset += 1;
            }
        }
        if reset == 0 {
            return;
        }
        warn!("upload: pulled {reset} deadline(s) back to {horizon}");
    }

    /// The minimum effective deadline over all entries, where a `None`
    /// deadline counts as `DateTime::<Utc>::MIN_UTC`. Returns `None` only when
    /// the queue is empty.
    pub fn earliest_next_attempt(&self) -> Option<DateTime<Utc>> {
        self.jobs
            .iter()
            .map(|entry| entry.next_attempt_at.unwrap_or(DateTime::<Utc>::MIN_UTC))
            .min()
    }

    /// Evict at most one entry so a new job can be admitted. A no-op below
    /// capacity, and a no-op on an empty queue — which is what makes
    /// `capacity == 0` degenerate to capacity 1 instead of evicting the job it
    /// just admitted.
    fn make_room(&mut self, now: DateTime<Utc>) {
        if self.jobs.len() < self.capacity {
            return;
        }
        let Some(idx) = self.victim_index() else {
            return;
        };
        let Some(victim) = self.jobs.remove(idx) else {
            return;
        };
        error!(
            "upload: queue is full (capacity {}); evicting file {} (rule {}, digest {}) after {} \
             attempt(s), first observed {} minute(s) ago; it will never be uploaded",
            self.capacity,
            victim.job.file,
            victim.job.file_rule_id,
            victim.job.digest,
            victim.attempts,
            (now - victim.job.first_observed_at).num_minutes(),
        );
    }

    /// The entry to evict: the most-failed one, ties broken by the oldest
    /// `first_observed_at`. `Reverse` inverts the secondary key so the maximum
    /// over the pair picks the *earliest* `first_observed_at`.
    fn victim_index(&self) -> Option<usize> {
        self.jobs
            .iter()
            .enumerate()
            .max_by_key(|(_, entry)| {
                (
                    entry.attempts,
                    std::cmp::Reverse(entry.job.first_observed_at),
                )
            })
            .map(|(idx, _)| idx)
    }

    async fn persist(&mut self) {
        let Some(snapshot_file) = self.snapshot_file.as_mut() else {
            return;
        };
        let snapshot = QueueSnapshot {
            entries: self.jobs.iter().cloned().collect(),
        };
        if let Err(err) = snapshot_file.patch(snapshot).await {
            warn!("upload: failed to persist upload queue: {err}");
        }
    }
}
