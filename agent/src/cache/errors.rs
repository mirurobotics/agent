use crate::errors::Trace;
use crate::filesys::errors::FileSysErr;

#[derive(Debug, thiserror::Error)]
#[error("unable to find cache element: {msg}")]
pub struct CacheElementNotFound {
    pub msg: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for CacheElementNotFound {}

#[derive(Debug, thiserror::Error)]
pub struct FoundTooManyCacheElements {
    pub expected_count: usize,
    pub actual_count: usize,
    pub filter_name: String,
    pub trace: Box<Trace>,
}

impl std::fmt::Display for FoundTooManyCacheElements {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "expected to find {} elements when filtering by '{}' but found {}",
            self.expected_count, self.filter_name, self.actual_count
        )
    }
}

impl crate::errors::Error for FoundTooManyCacheElements {}

#[derive(Debug, thiserror::Error)]
#[error("cannot overwrite cache element: {key}")]
pub struct CannotOverwriteCacheElement {
    pub key: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for CannotOverwriteCacheElement {}

#[derive(Debug, thiserror::Error)]
#[error("failed to send actor message: {source:?}")]
pub struct SendActorMessageErr {
    #[source]
    pub source: Box<dyn std::error::Error + Send + Sync>,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for SendActorMessageErr {}

#[derive(Debug, thiserror::Error)]
#[error("failed to receive actor message: {source}")]
pub struct ReceiveActorMessageErr {
    #[source]
    pub source: Box<dyn std::error::Error + Send + Sync>,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for ReceiveActorMessageErr {}

#[derive(Debug, thiserror::Error)]
pub enum CacheErr {
    #[error(transparent)]
    CacheElementNotFound(CacheElementNotFound),
    #[error(transparent)]
    CannotOverwriteCacheElement(CannotOverwriteCacheElement),
    #[error(transparent)]
    FileSysErr(FileSysErr),
    #[error(transparent)]
    FoundTooManyCacheElements(FoundTooManyCacheElements),
    #[error(transparent)]
    SendActorMessageErr(SendActorMessageErr),
    #[error(transparent)]
    ReceiveActorMessageErr(ReceiveActorMessageErr),
}

impl From<FileSysErr> for CacheErr {
    fn from(e: FileSysErr) -> Self {
        Self::FileSysErr(e)
    }
}

crate::impl_error!(CacheErr {
    CacheElementNotFound,
    CannotOverwriteCacheElement,
    FileSysErr,
    FoundTooManyCacheElements,
    SendActorMessageErr,
    ReceiveActorMessageErr,
});
