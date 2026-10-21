// standard crates
use std::collections::VecDeque;

// internal crates
use crate::filesys::FileSysErr;
use crate::queue::{
    errors::{EnqueueErr, QueueFullErr},
    job::QueueJob,
    snapshot::{QueueEntry, QueueSnapshot, QueueSnapshotFile},
};

// external crates
use chrono::{DateTime, Utc};
use tracing::{info, warn};
use uuid::Uuid;

/// What [`FileQueue::enqueue`] does when the snapshot write fails. Chosen once per
/// queue, by what the queue promises the producers that fill it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Persistence {
    /// Keep the entry in memory and log the failure; the next successful
    /// write saves it. For producers that never hear back, such as a scanner.
    BestEffort,
    /// Admit the entry only once the snapshot holding it is written; a failed
    /// write rolls it back and is returned. For producers told their job is
    /// safe, such as an HTTP caller answered 201.
    Durable,
}

/// In-memory job queue with an optional single-file snapshot.
///
/// A worker takes a job in three steps: `next_ready` selects an entry without
/// taking it out, the worker attempts the job, and then it settles the entry
/// with `ack` (done with it, success or give up) or `retry` (try again later).
/// The entry stays queued, and on disk, until it is settled, so a crash
/// mid-attempt never loses a job.
///
/// Every mutation (`enqueue`, `ack`, `retry`, `reset_invalid_deadlines`)
/// persists as its last act, so there is exactly one persist implementation
/// and no caller can forget to call it. Selection (`next_ready`,
/// `count_ready`) never mutates and never persists.
pub struct FileQueue<J: QueueJob> {
    entries: VecDeque<QueueEntry<J>>,
    capacity: usize,
    snapshot_file: Option<QueueSnapshotFile<J>>,
    persistence: Persistence,
}

impl<J: QueueJob> FileQueue<J> {
    /// An in-memory-only queue with no disk persistence.
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            capacity,
            snapshot_file: None,
            persistence: Persistence::BestEffort,
        }
    }

    pub fn from_snapshot(
        capacity: usize,
        snapshot_file: QueueSnapshotFile<J>,
        persistence: Persistence,
    ) -> Self {
        let entries = snapshot_file.read().entries.iter().cloned().collect();
        Self {
            entries,
            capacity,
            snapshot_file: Some(snapshot_file),
            persistence,
        }
    }

    // ================ Accessors ================ //

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn is_ready(entry: &QueueEntry<J>, now: DateTime<Utc>) -> bool {
        entry.job.due_at() <= now && entry.next_attempt_at.is_none_or(|at| at <= now)
    }

    pub fn count_ready(&self, now: DateTime<Utc>) -> usize {
        self.entries
            .iter()
            .filter(|entry| Self::is_ready(entry, now))
            .count()
    }

    pub fn next_ready(&self, now: DateTime<Utc>) -> Option<QueueEntry<J>> {
        let entry = self
            .entries
            .iter()
            .find(|entry| Self::is_ready(entry, now))?;
        info!(
            "{}: job selected; queue length {}",
            J::LABEL,
            self.entries.len()
        );
        Some(entry.clone())
    }

    pub fn earliest_next_attempt(&self) -> Option<DateTime<Utc>> {
        self.entries
            .iter()
            .map(|entry| entry.next_attempt_at.unwrap_or(DateTime::<Utc>::MIN_UTC))
            .min()
    }

    async fn verify_capacity(&mut self, job: &J) -> Result<(), QueueFullErr> {
        if self.entries.len() >= self.capacity {
            warn!(
                "{} queue is full (capacity {}); rejecting {}",
                J::LABEL,
                self.capacity,
                job.name()
            );
            return Err(QueueFullErr::new(J::LABEL, self.capacity, job.name()));
        }
        Ok(())
    }

    // ================ Mutators ================ //

    /// Push a new job at the tail under a fresh id, which is returned. A full
    /// queue rejects it with [`EnqueueErr::QueueFullErr`] and persists nothing.
    /// A failed snapshot write is handled as the queue's [`Persistence`] says:
    /// only a durable queue returns it, as [`EnqueueErr::FileSysErr`].
    pub async fn enqueue(&mut self, job: J) -> Result<Uuid, EnqueueErr> {
        self.verify_capacity(&job).await?;
        let id = Uuid::new_v4();
        self.push(id, job);
        if let Err(err) = self.persist().await {
            if self.persistence == Persistence::Durable {
                self.take(id);
                return Err(EnqueueErr::FileSysErr(err));
            }
        }
        self.log_enqueued();
        Ok(id)
    }

    fn push(&mut self, id: Uuid, job: J) {
        self.entries.push_back(QueueEntry {
            id,
            job,
            attempts: 0,
            next_attempt_at: None,
        });
    }

    fn log_enqueued(&self) {
        info!(
            "{}: job enqueued; queue length {}",
            J::LABEL,
            self.entries.len()
        );
    }

    /// Settle the entry with `id` as finished, whether the job succeeded or
    /// the worker gave up on it, and persist the shortened queue. This is the
    /// only point at which a job leaves durable storage. The entry need not be
    /// at the head, since `retry` rotates entries to the tail. Unknown ids are
    /// ignored.
    pub async fn ack(&mut self, id: Uuid) -> Option<QueueEntry<J>> {
        let entry = self.take(id)?;
        let _ = self.persist().await;
        info!(
            "{}: job acked; queue length {}",
            J::LABEL,
            self.entries.len()
        );
        Some(entry)
    }

    /// Settle the entry with `id` for another try: record its `attempts` so
    /// far, defer it until `next_attempt_at`, and rotate it to the tail. The
    /// job itself is unchanged. The entry is already queued, so this is not
    /// capacity-gated and cannot change [`Self::len`]. Returns `false`, and
    /// persists nothing, for an unknown id.
    pub async fn retry(&mut self, id: Uuid, attempts: u32, next_attempt_at: DateTime<Utc>) -> bool {
        let Some(mut entry) = self.take(id) else {
            return false;
        };
        entry.attempts = attempts;
        entry.next_attempt_at = Some(next_attempt_at);
        self.entries.push_back(entry);
        let _ = self.persist().await;
        info!(
            "{}: job queued for retry; queue length {}",
            J::LABEL,
            self.entries.len()
        );
        true
    }

    fn take(&mut self, id: Uuid) -> Option<QueueEntry<J>> {
        let idx = self.entries.iter().position(|entry| entry.id == id)?;
        self.entries.remove(idx)
    }

    /// Pull every `next_attempt_at` strictly beyond `horizon` back to it, so no
    /// entry can be deferred past a deadline the caller considers reachable.
    /// A no-op when every deadline is already inside the horizon, so nothing
    /// is persisted in that case.
    pub async fn reset_invalid_deadlines(&mut self, horizon: DateTime<Utc>) {
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
        let _ = self.persist().await;
        warn!("{}: pulled {reset} deadline(s) back to {horizon}", J::LABEL);
    }

    /// The sole writer to disk, called by every mutator as its last act. A
    /// failed write is logged and returned. Only a durable `enqueue` acts on
    /// the error; every other caller keeps the in-memory queue authoritative.
    /// A queue without a snapshot file has nothing to write and always succeeds.
    async fn persist(&mut self) -> Result<(), FileSysErr> {
        let Some(snapshot_file) = self.snapshot_file.as_mut() else {
            return Ok(());
        };
        let snapshot = QueueSnapshot {
            entries: self.entries.iter().cloned().collect(),
        };
        let result = snapshot_file.patch(snapshot).await;
        if let Err(err) = &result {
            warn!(
                "{label}: failed to persist {label} queue: {err}",
                label = J::LABEL
            );
        }
        result
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
        let mut queue = FileQueue::<Job>::new(4);
        queue.enqueue(job("a.log")).await.unwrap();
        queue.enqueue(job("b.log")).await.unwrap();

        assert_eq!(queue.entries(), vec![job("a.log"), job("b.log")]);

        let entries = queue.queue_entries();
        let jobs: Vec<Job> = entries.iter().map(|entry| entry.job.clone()).collect();
        assert_eq!(jobs, queue.entries());
        assert_ne!(entries[0].id, entries[1].id);
    }
}
