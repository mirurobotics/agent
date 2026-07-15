// internal crates
use crate::errors::Trace;

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
}

crate::impl_error!(UploadErr {
    QueueFullErr,
    ExecutorErr
});

/// Wraps any concrete error as an [`UploadErr::ExecutorErr`], the single
/// error surface the actor sees from executor and transfer failures.
pub(crate) fn executor_err<E>(source: E) -> UploadErr
where
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    UploadErr::ExecutorErr(ExecutorErr {
        source: source.into(),
        trace: crate::trace!(),
    })
}
