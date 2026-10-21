// internal crates
use crate::errors::Trace;

/// Returned (inside [`EnqueueErr`]) by [`super::FileQueue::enqueue`], and by
/// [`super::DirQueue::enqueue`], when the queue is at capacity. Nothing was written and nothing already queued
/// was dropped.
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

/// The item's file would exceed the per-item size cap. Nothing was written;
/// retrying the same item fails the same way.
#[derive(Debug, thiserror::Error)]
#[error("{label} item of {item_bytes} bytes exceeds the {max_item_bytes}-byte limit")]
pub struct ItemTooLargeErr {
    pub label: &'static str,
    pub item_bytes: u64,
    pub max_item_bytes: u64,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for ItemTooLargeErr {}

#[derive(Debug, thiserror::Error)]
#[error("failed to serialize {label} item {name}: {source}")]
pub struct SerializeItemErr {
    pub label: &'static str,
    pub name: String,
    #[source]
    pub source: serde_json::Error,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for SerializeItemErr {}

/// Everything [`super::DirQueue`] can fail with.
#[derive(Debug, thiserror::Error)]
pub enum DirQueueErr {
    #[error(transparent)]
    QueueFullErr(QueueFullErr),
    #[error(transparent)]
    ItemTooLargeErr(ItemTooLargeErr),
    #[error(transparent)]
    SerializeItemErr(SerializeItemErr),
    #[error(transparent)]
    EnqueueErr(EnqueueErr),
    #[error(transparent)]
    FileSysErr(crate::filesys::FileSysErr),
}

impl From<crate::filesys::FileSysErr> for DirQueueErr {
    fn from(e: crate::filesys::FileSysErr) -> Self {
        Self::FileSysErr(e)
    }
}

/// A full queue surfaces as [`DirQueueErr::QueueFullErr`]; any other enqueue
/// error stays wrapped.
impl From<EnqueueErr> for DirQueueErr {
    fn from(e: EnqueueErr) -> Self {
        match e {
            EnqueueErr::QueueFullErr(e) => Self::QueueFullErr(e),
            other => Self::EnqueueErr(other),
        }
    }
}

crate::impl_error!(DirQueueErr {
    QueueFullErr,
    ItemTooLargeErr,
    SerializeItemErr,
    EnqueueErr,
    FileSysErr,
});
