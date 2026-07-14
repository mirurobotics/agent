// internal crates
use crate::errors::Trace;

pub type SendActorMessageErr = crate::cache::errors::SendActorMessageErr;
pub type ReceiveActorMessageErr = crate::cache::errors::ReceiveActorMessageErr;

#[derive(Debug, thiserror::Error)]
#[error("upload queue is full (capacity {capacity}); rejected job for file {file}")]
pub struct QueueFullErr {
    pub capacity: usize,
    pub file: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for QueueFullErr {}

#[derive(Debug, thiserror::Error)]
#[error("upload executor error: {source}")]
pub struct ExecutorErr {
    #[source]
    pub source: Box<dyn std::error::Error + Send + Sync>,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for ExecutorErr {}

#[derive(Debug, thiserror::Error)]
pub enum UploadErr {
    #[error(transparent)]
    QueueFullErr(QueueFullErr),
    #[error(transparent)]
    ExecutorErr(ExecutorErr),
    #[error(transparent)]
    SendActorMessageErr(SendActorMessageErr),
    #[error(transparent)]
    ReceiveActorMessageErr(ReceiveActorMessageErr),
}

crate::impl_error!(UploadErr {
    QueueFullErr,
    ExecutorErr,
    SendActorMessageErr,
    ReceiveActorMessageErr
});
