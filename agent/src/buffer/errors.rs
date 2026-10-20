// internal crates
use crate::errors::Trace;
use crate::filesys::FileSysErr;
use crate::queue::EnqueueErr;

/// The buffer is at its item or byte cap. Nothing was written and nothing
/// already buffered was dropped; the caller should retry once the buffer
/// drains. Callers that expose this over HTTP wrap it with their own code.
#[derive(Debug, thiserror::Error)]
#[error(
    "{label} buffer is full ({items}/{max_items} items, {bytes}/{max_bytes} bytes); rejected an item of {item_bytes} bytes"
)]
pub struct BufferFullErr {
    pub label: &'static str,
    pub item_bytes: u64,
    pub items: usize,
    pub max_items: usize,
    pub bytes: u64,
    pub max_bytes: u64,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for BufferFullErr {}

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
    SerializeItemErr,
    EnqueueErr,
    FileSysErr,
});
