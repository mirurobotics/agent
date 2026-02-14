use crate::cache::errors::CacheErr;
use crate::crypt::errors::CryptErr;
use crate::errors::Trace;
use crate::filesys::errors::FileSysErr;

#[derive(Debug, thiserror::Error)]
#[error("device is not activated: {msg}")]
pub struct DeviceNotActivatedErr {
    pub msg: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for DeviceNotActivatedErr {}

#[derive(Debug, thiserror::Error)]
#[error("failed to join handle: {source}")]
pub struct JoinHandleErr {
    #[source]
    pub source: Box<dyn std::error::Error + Send + Sync>,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for JoinHandleErr {}

#[derive(Debug, thiserror::Error)]
pub struct PruneCacheErrs {
    pub sources: Vec<CacheErr>,
    pub trace: Box<Trace>,
}

impl std::fmt::Display for PruneCacheErrs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "failed to prune caches: {:?}", self.sources)
    }
}

impl crate::errors::Error for PruneCacheErrs {}

#[derive(Debug, thiserror::Error)]
pub enum StorageErr {
    #[error(transparent)]
    DeviceNotActivatedErr(DeviceNotActivatedErr),
    #[error(transparent)]
    PruneCacheErrs(PruneCacheErrs),
    #[error(transparent)]
    CacheErr(CacheErr),
    #[error(transparent)]
    CryptErr(CryptErr),
    #[error(transparent)]
    FileSysErr(FileSysErr),
    #[error(transparent)]
    JoinHandleErr(JoinHandleErr),
}

impl From<CacheErr> for StorageErr {
    fn from(e: CacheErr) -> Self {
        Self::CacheErr(e)
    }
}

impl From<CryptErr> for StorageErr {
    fn from(e: CryptErr) -> Self {
        Self::CryptErr(e)
    }
}

impl From<FileSysErr> for StorageErr {
    fn from(e: FileSysErr) -> Self {
        Self::FileSysErr(e)
    }
}

crate::impl_error!(StorageErr {
    DeviceNotActivatedErr,
    PruneCacheErrs,
    CacheErr,
    CryptErr,
    FileSysErr,
    JoinHandleErr,
});
