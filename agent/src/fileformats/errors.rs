// internal crates
use crate::filesys;

#[derive(Debug, thiserror::Error)]
pub enum FileFormatsErr {
    #[error(transparent)]
    FileSysErr(filesys::FileSysErr),
}

impl From<filesys::FileSysErr> for FileFormatsErr {
    fn from(e: filesys::FileSysErr) -> Self {
        Self::FileSysErr(e)
    }
}

crate::impl_error!(FileFormatsErr { FileSysErr });
