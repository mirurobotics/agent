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
    /// True only for a definitive client error (4xx excluding 408, 429,
    /// and 401) returned by the upload's own backend request. Permanent
    /// failures are dropped by the actor instead of retried.
    pub permanent: bool,
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

impl UploadErr {
    /// Whether retrying this error can never succeed.
    pub fn is_permanent(&self) -> bool {
        matches!(self, Self::ExecutorErr(e) if e.permanent)
    }
}

crate::impl_error!(UploadErr {
    QueueFullErr,
    ExecutorErr,
    SendActorMessageErr,
    ReceiveActorMessageErr
});

/// Whether `e` is a permanent client error: a definitive 4xx response
/// that is not a timeout (408), rate limit (429), or stale-token (401)
/// condition. Network connection errors and non-HTTP errors (which
/// default to a 500 status) are never permanent.
pub fn is_permanent<E: crate::errors::Error>(e: &E) -> bool {
    if e.is_network_conn_err() {
        return false;
    }
    let status = e.http_status();
    status.is_client_error()
        && status != HTTPCode::REQUEST_TIMEOUT
        && status != HTTPCode::TOO_MANY_REQUESTS
        && status != HTTPCode::UNAUTHORIZED
}

/// Wraps any concrete error as an [`UploadErr::ExecutorErr`], the single
/// error surface the actor sees from executor and transfer failures.
pub(crate) fn executor_err<E>(source: E) -> UploadErr
where
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    UploadErr::ExecutorErr(ExecutorErr {
        source: source.into(),
        permanent: false,
        trace: crate::trace!(),
    })
}

/// Wraps an error from the upload's own backend request, classifying
/// permanence from the concrete type before erasing it.
pub(crate) fn backend_err<E>(source: E) -> UploadErr
where
    E: crate::errors::Error + Send + Sync + 'static,
{
    let permanent = is_permanent(&source);
    UploadErr::ExecutorErr(ExecutorErr {
        source: Box::new(source),
        permanent,
        trace: crate::trace!(),
    })
}
