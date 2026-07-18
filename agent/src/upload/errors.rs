// internal crates
use crate::errors::{HTTPCode, Trace};

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
    /// Some(status) when the wrapped failure was classified terminal at wrap time.
    pub terminal_status: Option<HTTPCode>,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for ExecutorErr {
    fn is_terminal(&self) -> bool {
        self.terminal_status.is_some()
    }
}

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

impl UploadErr {
    /// The backend HTTP status behind a terminal failure, if any.
    pub fn terminal_status(&self) -> Option<HTTPCode> {
        match self {
            Self::ExecutorErr(e) => e.terminal_status,
            _ => None,
        }
    }
}

/// Wraps any concrete error as an [`UploadErr::ExecutorErr`], the single
/// error surface the actor sees from executor and transfer failures.
pub(crate) fn executor_err<E>(source: E) -> UploadErr
where
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    UploadErr::ExecutorErr(ExecutorErr {
        source: source.into(),
        terminal_status: None,
        trace: crate::trace!(),
    })
}

/// Like [`executor_err`], but captures the terminal classification (and
/// the backend status behind it) before the type is erased.
pub(crate) fn classified_executor_err<E>(source: E) -> UploadErr
where
    E: crate::errors::Error + Send + Sync + 'static,
{
    let terminal_status = source.is_terminal().then_some(source.http_status());
    UploadErr::ExecutorErr(ExecutorErr {
        source: Box::new(source),
        terminal_status,
        trace: crate::trace!(),
    })
}
