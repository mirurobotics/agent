use crate::cache::errors::CacheErr;
use crate::crud::errors::CrudErr;
use crate::filesys::errors::FileSysErr;
use crate::http::errors::HTTPErr;
use crate::models::errors::ModelsErr;
use crate::storage::errors::StorageErr;
use crate::sync::errors::SyncErr;

#[derive(Debug, thiserror::Error)]
pub enum ServiceErr {
    #[error(transparent)]
    CacheErr(CacheErr),
    #[error(transparent)]
    CrudErr(CrudErr),
    #[error(transparent)]
    FileSysErr(FileSysErr),
    #[error(transparent)]
    ModelsErr(ModelsErr),
    #[error(transparent)]
    StorageErr(StorageErr),
    #[error(transparent)]
    HTTPErr(HTTPErr),
    #[error(transparent)]
    SyncErr(SyncErr),
}

impl From<CacheErr> for ServiceErr {
    fn from(e: CacheErr) -> Self {
        Self::CacheErr(e)
    }
}

impl From<CrudErr> for ServiceErr {
    fn from(e: CrudErr) -> Self {
        Self::CrudErr(e)
    }
}

impl From<FileSysErr> for ServiceErr {
    fn from(e: FileSysErr) -> Self {
        Self::FileSysErr(e)
    }
}

impl From<ModelsErr> for ServiceErr {
    fn from(e: ModelsErr) -> Self {
        Self::ModelsErr(e)
    }
}

impl From<StorageErr> for ServiceErr {
    fn from(e: StorageErr) -> Self {
        Self::StorageErr(e)
    }
}

impl From<HTTPErr> for ServiceErr {
    fn from(e: HTTPErr) -> Self {
        Self::HTTPErr(e)
    }
}

impl From<SyncErr> for ServiceErr {
    fn from(e: SyncErr) -> Self {
        Self::SyncErr(e)
    }
}

crate::impl_error!(ServiceErr {
    CacheErr,
    CrudErr,
    FileSysErr,
    ModelsErr,
    StorageErr,
    HTTPErr,
    SyncErr,
});
