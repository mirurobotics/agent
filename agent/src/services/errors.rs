use crate::cache;
use crate::filesys;
use crate::http;
use crate::models;
use crate::storage::StorageErr;
use crate::sync;

#[derive(Debug, thiserror::Error)]
pub enum ServiceErr {
    #[error(transparent)]
    CacheErr(cache::CacheErr),
    #[error(transparent)]
    FileSysErr(filesys::FileSysErr),
    #[error(transparent)]
    ModelsErr(models::ModelsErr),
    #[error(transparent)]
    StorageErr(StorageErr),
    #[error(transparent)]
    HTTPErr(http::HTTPErr),
    #[error(transparent)]
    SyncErr(sync::SyncErr),
}

impl From<cache::CacheErr> for ServiceErr {
    fn from(e: cache::CacheErr) -> Self {
        Self::CacheErr(e)
    }
}

impl From<filesys::FileSysErr> for ServiceErr {
    fn from(e: filesys::FileSysErr) -> Self {
        Self::FileSysErr(e)
    }
}

impl From<models::ModelsErr> for ServiceErr {
    fn from(e: models::ModelsErr) -> Self {
        Self::ModelsErr(e)
    }
}

impl From<StorageErr> for ServiceErr {
    fn from(e: StorageErr) -> Self {
        Self::StorageErr(e)
    }
}

impl From<http::HTTPErr> for ServiceErr {
    fn from(e: http::HTTPErr) -> Self {
        Self::HTTPErr(e)
    }
}

impl From<sync::SyncErr> for ServiceErr {
    fn from(e: sync::SyncErr) -> Self {
        Self::SyncErr(e)
    }
}

crate::impl_error!(ServiceErr {
    CacheErr,
    FileSysErr,
    ModelsErr,
    StorageErr,
    HTTPErr,
    SyncErr,
});
