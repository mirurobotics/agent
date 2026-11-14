// internal crates
use crate::errors::Trace;

// external crates
use uuid::Uuid;

/// Returned (inside [`EnqueueErr`]) by [`super::FileQueue::enqueue`] when the
/// queue is at capacity. Nothing was written and nothing already queued was
/// dropped.
/// Workers wrap this in their own error enum via `From`.
#[derive(Debug, thiserror::Error)]
#[error("{label} queue is full (capacity {capacity}); rejected job for file {name}")]
pub struct QueueFullErr {
    pub label: &'static str,
    pub capacity: usize,
    pub name: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for QueueFullErr {}

impl QueueFullErr {
    pub fn new(label: &'static str, capacity: usize, name: impl Into<String>) -> Self {
        Self {
            label,
            capacity,
            name: name.into(),
            trace: crate::trace!(),
        }
    }
}

/// Returned by [`super::FileQueue::enqueue`]: either the queue is full or, for a
/// durable queue, the snapshot holding the new entry could not be written.
#[derive(Debug, thiserror::Error)]
pub enum EnqueueErr {
    #[error(transparent)]
    QueueFullErr(QueueFullErr),
    #[error(transparent)]
    FileSysErr(crate::filesys::FileSysErr),
}

impl From<QueueFullErr> for EnqueueErr {
    fn from(e: QueueFullErr) -> Self {
        Self::QueueFullErr(e)
    }
}

crate::impl_error!(EnqueueErr {
    QueueFullErr,
    FileSysErr,
});

/// Returned by [`super::FileQueue::update_job`] when no queued entry has the id.
#[derive(Debug, thiserror::Error)]
#[error("{label} queue has no entry {id}")]
pub struct EntryNotFoundErr {
    pub label: &'static str,
    pub id: Uuid,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for EntryNotFoundErr {}
