// internal crates
use crate::errors::Trace;
use crate::filesys::FileSysErr;
use crate::queue::EnqueueErr;

/// The buffer is at its item cap. Nothing was written and nothing already
/// buffered was dropped; the caller should retry once the buffer drains.
/// Callers that expose this over HTTP wrap it with their own code.
#[derive(Debug, thiserror::Error)]
#[error("{label} buffer is full ({items}/{max_items} items)")]
pub struct BufferFullErr {
    pub label: &'static str,
    pub items: usize,
    pub max_items: usize,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for BufferFullErr {}

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
#[error("failed to serialize {label} item {id}: {source}")]
pub struct SerializeItemErr {
    pub label: &'static str,
    pub id: uuid::Uuid,
    #[source]
    pub source: serde_json::Error,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for SerializeItemErr {}

#[derive(Debug, thiserror::Error)]
pub enum BufferErr {
    #[error(transparent)]
    BufferFullErr(BufferFullErr),
    #[error(transparent)]
    ItemTooLargeErr(ItemTooLargeErr),
    #[error(transparent)]
    SerializeItemErr(SerializeItemErr),
    #[error(transparent)]
    EnqueueErr(EnqueueErr),
    #[error(transparent)]
    FileSysErr(FileSysErr),
}

impl From<FileSysErr> for BufferErr {
    fn from(e: FileSysErr) -> Self {
        Self::FileSysErr(e)
    }
}

impl From<EnqueueErr> for BufferErr {
    fn from(e: EnqueueErr) -> Self {
        Self::EnqueueErr(e)
    }
}

crate::impl_error!(BufferErr {
    BufferFullErr,
    ItemTooLargeErr,
    SerializeItemErr,
    EnqueueErr,
    FileSysErr,
});
