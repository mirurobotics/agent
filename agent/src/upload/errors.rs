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
    pub is_terminal: bool,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for ExecutorErr {
    fn is_terminal(&self) -> bool {
        self.is_terminal
    }
}

#[derive(Debug, thiserror::Error)]
#[error("upload attempt for file {file} ({size} bytes) exceeded its {deadline:?} deadline")]
pub struct AttemptTimeoutErr {
    pub file: String,
    pub size: u64,
    pub deadline: std::time::Duration,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for AttemptTimeoutErr {}

#[derive(Debug, thiserror::Error)]
pub enum UploadErr {
    #[error(transparent)]
    QueueFullErr(QueueFullErr),
    #[error(transparent)]
    ExecutorErr(ExecutorErr),
    #[error(transparent)]
    AttemptTimeoutErr(AttemptTimeoutErr),
    #[error(transparent)]
    SendActorMessageErr(SendActorMessageErr),
    #[error(transparent)]
    ReceiveActorMessageErr(ReceiveActorMessageErr),
}

crate::impl_error!(UploadErr {
    QueueFullErr,
    ExecutorErr,
    AttemptTimeoutErr,
    SendActorMessageErr,
    ReceiveActorMessageErr
});

/// Wraps any concrete error as an [`UploadErr::ExecutorErr`], the single
/// error surface the actor sees from executor and transfer failures.
pub(crate) fn executor_err<E>(source: E) -> UploadErr
where
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    wrap_executor_err(source, false)
}

/// Wraps a Miru error while preserving its terminal classification.
pub(crate) fn classified_executor_err<E>(source: E) -> UploadErr
where
    E: crate::errors::Error + Send + Sync + 'static,
{
    let is_terminal = source.is_terminal();
    wrap_executor_err(source, is_terminal)
}

fn wrap_executor_err<E>(source: E, is_terminal: bool) -> UploadErr
where
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    UploadErr::ExecutorErr(ExecutorErr {
        source: source.into(),
        is_terminal,
        trace: crate::trace!(),
    })
}
