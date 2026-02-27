use crate::cache;
use crate::crypt;
use crate::errors::Trace;
use crate::filesys;

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
#[error("failed to prune caches: {sources:?}")]
pub struct PruneCacheErrs {
    pub sources: Vec<cache::CacheErr>,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for PruneCacheErrs {}

#[derive(Debug, thiserror::Error)]
pub enum StorageErr {
    #[error(transparent)]
    DeviceNotActivatedErr(DeviceNotActivatedErr),
    #[error(transparent)]
    PruneCacheErrs(PruneCacheErrs),
    #[error(transparent)]
    CacheErr(cache::CacheErr),
    #[error(transparent)]
    CryptErr(crypt::CryptErr),
    #[error(transparent)]
    FileSysErr(filesys::FileSysErr),
    #[error(transparent)]
    JoinHandleErr(JoinHandleErr),
}

impl From<cache::CacheErr> for StorageErr {
    fn from(e: cache::CacheErr) -> Self {
        Self::CacheErr(e)
    }
}

impl From<crypt::CryptErr> for StorageErr {
    fn from(e: crypt::CryptErr) -> Self {
        Self::CryptErr(e)
    }
}

impl From<filesys::FileSysErr> for StorageErr {
    fn from(e: filesys::FileSysErr) -> Self {
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
