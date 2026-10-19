// internal crates
use crate::errors::Trace;

/// Returned by [`super::Queue::enqueue`] when the queue is at capacity.
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

/// Returned by [`super::Queue::enqueue_durable`]: either the queue is full or
/// the snapshot holding the new entry could not be written.
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
