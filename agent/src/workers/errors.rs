// internal crates
use crate::scan::ScanErr;

#[derive(Debug, thiserror::Error)]
pub enum WorkerErr {
    #[error(transparent)]
    ScanErr(ScanErr),
}

impl From<ScanErr> for WorkerErr {
    fn from(e: ScanErr) -> Self {
        Self::ScanErr(e)
    }
}

crate::impl_error!(WorkerErr {
    ScanErr,
});
