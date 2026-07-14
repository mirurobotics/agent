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
pub enum UploadErr {
    #[error(transparent)]
    FileSysErr(#[from] crate::filesys::FileSysErr),
    #[error(transparent)]
    QueueFullErr(QueueFullErr),
}

crate::impl_error!(UploadErr {
    FileSysErr,
    QueueFullErr
});
