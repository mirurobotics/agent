// standard crates
use std::collections::VecDeque;

// internal crates
use crate::filesys::state_file::SingleThreadStateFile;
use crate::models::Patch;

// external crates
use chrono::{DateTime, Utc};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tracing::{info, warn};
use uuid::Uuid;

/// Everything a [`Queue`] needs to know about the job it carries.
pub trait QueueJob:
    Clone + std::fmt::Debug + PartialEq + Serialize + DeserializeOwned + Send + Sync + 'static
{
    /// Error returned by [`Queue::enqueue`] when the queue is at capacity.
    type QueueFullErr;

    /// Log prefix, e.g. `"upload"` or `"delete"`.
    const LABEL: &'static str;

    /// Earliest instant this job may be actioned. Jobs actionable on arrival
    /// return `DateTime::<Utc>::MIN_UTC`.
    fn due_at(&self) -> DateTime<Utc>;

    /// The file this job concerns, for logs and the capacity error.
    fn file(&self) -> String;

    /// Build the capacity rejection. Implementors call `crate::trace!()` in
    /// their own body so the trace records the implementor's location rather
    /// than this shared code.
    fn queue_full_err(capacity: usize, file: String) -> Self::QueueFullErr;
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueueEntry<J> {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    pub job: J,
    #[serde(default)]
    pub attempts: u32,
    /// Earliest instant this entry is eligible to be popped; `None` means
    /// "eligible now".
    #[serde(default)]
    pub next_attempt_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueueSnapshot<J> {
    pub entries: Vec<QueueEntry<J>>,
}

// hand-written rather than derived: `#[derive(Default)]` would add a spurious
// `J: Default` bound that neither job type satisfies.
impl<J> Default for QueueSnapshot<J> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

impl<J> Patch<QueueSnapshot<J>> for QueueSnapshot<J> {
    fn patch(&mut self, patch: QueueSnapshot<J>) {
        *self = patch;
    }
}

/// The queue's persistence handle: an atomically-written JSON snapshot with an
/// in-memory cache. Mirrors the scanner's `ScanSnapshotFile`.
pub type QueueSnapshotFile<J> = SingleThreadStateFile<QueueSnapshot<J>, QueueSnapshot<J>>;

/// In-memory job queue with an optional snapshot. Every mutation (`enqueue`,
/// `remove`, `requeue`) persists as its last act, so there is exactly one
/// persist implementation and no caller can forget to call it. Selection
/// (`next_ready`, `count_ready`) never mutates and never persists.
pub struct Queue<J: QueueJob> {
    entries: VecDeque<QueueEntry<J>>,
    capacity: usize,
    snapshot_file: Option<QueueSnapshotFile<J>>,
}

impl<J: QueueJob> Queue<J> {
    /// An in-memory-only queue with no disk persistence.
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            capacity,
            snapshot_file: None,
        }
    }

    /// A disk-backed queue: `snapshot_file`'s persisted entries seed the initial
    /// backlog, and every subsequent mutation is written back. Persisted entries are
    /// loaded in full even if they exceed `capacity`; capacity only gates new enqueues,
    /// so an over-capacity backlog simply drains before it accepts more.
    pub fn from_snapshot(capacity: usize, snapshot_file: QueueSnapshotFile<J>) -> Self {
        let entries = snapshot_file.read().entries.iter().cloned().collect();
        Self {
            entries,
            capacity,
            snapshot_file: Some(snapshot_file),
        }
    }

    /// The number of queued entries, including one currently in flight: a
    /// selected entry stays queued until it resolves.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Push a new job at the tail under a fresh id. When the queue is full the
    /// enqueue is rejected with `J::QueueFullErr` and nothing is persisted.
    pub async fn enqueue(&mut self, job: J) -> Result<(), J::QueueFullErr> {
        self.verify_capacity(&job).await?;
        self.entries.push_back(QueueEntry {
            id: Uuid::new_v4(),
            job,
            attempts: 0,
            next_attempt_at: None,
        });
        self.persist().await;
        info!(
            "{}: job enqueued; queue length {}",
            J::LABEL,
            self.entries.len()
        );
        Ok(())
    }

    /// Drop the entry with `id` and persist the shortened queue. This is the
    /// only point at which a job leaves durable storage, so call it only once
    /// the worker has resolved the entry. Unknown ids are ignored.
    pub async fn remove(&mut self, id: Uuid) -> Option<QueueEntry<J>> {
        if let Some(entry) = self.remove_impl(id) {
            self.persist().await;
            info!(
                "{}: job removed; queue length {}",
                J::LABEL,
                self.entries.len()
            );
            return Some(entry);
        }
        None
    }

    fn remove_impl(&mut self, id: Uuid) -> Option<QueueEntry<J>> {
        let idx = self.entries.iter().position(|entry| entry.id == id)?;
        self.entries.remove(idx)
    }

    /// Move `entry` to the tail, replacing whatever is queued under its id so
    /// the caller's updated attempt count and backoff deadline are persisted.
    /// This is a rotation of an entry the queue already holds, not an
    /// admission, so it is not capacity-gated and cannot change [`Self::len`].
    pub async fn requeue(&mut self, entry: QueueEntry<J>) {
        self.remove_impl(entry.id);
        self.entries.push_back(entry);
        self.persist().await;
        info!(
            "{}: job requeued; queue length {}",
            J::LABEL,
            self.entries.len()
        );
    }

    /// Return a clone of the first entry that is due at `now` and whose
    /// `next_attempt_at` is `None` or `<= now`. The entry is deliberately LEFT
    /// in the queue — and so in the persisted snapshot — until the caller
    /// passes its id back to [`Self::remove`] or hands the entry to
    /// [`Self::requeue`]. That is what makes the queue at-least-once: work cut
    /// short by a crash, a kill, or a shutdown is still on disk at the next
    /// boot and is retried. Returns `None` when no entry is eligible.
    pub fn next_ready(&self, now: DateTime<Utc>) -> Option<QueueEntry<J>> {
        let entry = self
            .entries
            .iter()
            .find(|entry| Self::is_ready(entry, now))?;
        info!(
            "{}: job dequeued; queue length {}",
            J::LABEL,
            self.entries.len()
        );
        Some(entry.clone())
    }

    /// How many entries are due at `now`. This is a sweep's loop budget — one
    /// visit per due entry — not an idle accessor.
    pub fn count_ready(&self, now: DateTime<Utc>) -> usize {
        self.entries
            .iter()
            .filter(|entry| Self::is_ready(entry, now))
            .count()
    }

    fn is_ready(entry: &QueueEntry<J>, now: DateTime<Utc>) -> bool {
        entry.job.due_at() <= now && entry.next_attempt_at.is_none_or(|at| at <= now)
    }

    /// Pull every `next_attempt_at` strictly beyond `horizon` back to it, so no
    /// entry can be deferred past a deadline the caller considers reachable.
    pub fn reset_invalid_deadlines(&mut self, horizon: DateTime<Utc>) {
        let mut reset = 0;
        for entry in self.entries.iter_mut() {
            if entry.next_attempt_at.is_some_and(|at| at > horizon) {
                entry.next_attempt_at = Some(horizon);
                reset += 1;
            }
        }
        if reset == 0 {
            return;
        }
        warn!("{}: pulled {reset} deadline(s) back to {horizon}", J::LABEL);
    }

    /// The minimum effective deadline over all entries, where a `None`
    /// deadline counts as `DateTime::<Utc>::MIN_UTC`. Returns `None` only when
    /// the queue is empty.
    pub fn earliest_next_attempt(&self) -> Option<DateTime<Utc>> {
        self.entries
            .iter()
            .map(|entry| entry.next_attempt_at.unwrap_or(DateTime::<Utc>::MIN_UTC))
            .min()
    }

    async fn verify_capacity(&mut self, job: &J) -> Result<(), J::QueueFullErr> {
        if self.entries.len() >= self.capacity {
            warn!(
                "{} queue is full (capacity {}); rejecting job for file {}",
                J::LABEL,
                self.capacity,
                job.file()
            );
            return Err(J::queue_full_err(self.capacity, job.file()));
        }
        Ok(())
    }

    /// The sole writer to disk. Called by every mutator as its last act.
    async fn persist(&mut self) {
        let Some(snapshot_file) = self.snapshot_file.as_mut() else {
            return;
        };
        let snapshot = QueueSnapshot {
            entries: self.entries.iter().cloned().collect(),
        };
        if let Err(err) = snapshot_file.patch(snapshot).await {
            warn!(
                "{label}: failed to persist {label} queue: {err}",
                label = J::LABEL
            );
        }
    }

    /// The queued jobs, oldest enqueue first (test observability only).
    #[cfg(test)]
    pub(crate) fn entries(&self) -> Vec<J> {
        self.entries.iter().map(|entry| entry.job.clone()).collect()
    }

    /// The queued entries, oldest enqueue first (test observability only).
    #[cfg(test)]
    pub(crate) fn queue_entries(&self) -> Vec<QueueEntry<J>> {
        self.entries.iter().cloned().collect()
    }
}

/// The queue's behavior is covered by the integration suite. Only the two
/// `#[cfg(test)]` accessors are covered here, since they are invisible outside
/// the crate.
#[cfg(test)]
mod tests {
    // internal crates
    use super::*;
    use crate::data_uploads::retention::job::Job;
    use crate::filesys::File;

    fn job(name: &str) -> Job {
        Job {
            file: File::new(format!("/data/{name}")),
            size: 42,
            digest: format!("sha256:{name}"),
            mtime: DateTime::from_timestamp(900, 0).unwrap(),
            first_observed_at: DateTime::from_timestamp(1000, 0).unwrap(),
            last_observed_at: DateTime::from_timestamp(1000, 0).unwrap(),
            ttl_secs: 0,
            file_rule_id: "file_rule_1".to_string(),
            deployment_id: "dpl_1".to_string(),
        }
    }

    #[tokio::test]
    async fn accessors_expose_the_queued_entries_in_order() {
        let mut queue = Queue::<Job>::new(4);
        queue.enqueue(job("a.log")).await.unwrap();
        queue.enqueue(job("b.log")).await.unwrap();

        assert_eq!(queue.entries(), vec![job("a.log"), job("b.log")]);

        let entries = queue.queue_entries();
        let jobs: Vec<Job> = entries.iter().map(|entry| entry.job.clone()).collect();
        assert_eq!(jobs, queue.entries());
        assert_ne!(entries[0].id, entries[1].id);
    }
}
