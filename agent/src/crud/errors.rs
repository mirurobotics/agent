use crate::cache::errors::CacheErr;
use crate::storage::errors::StorageErr;

#[derive(Debug, thiserror::Error)]
pub enum CrudErr {
    #[error(transparent)]
    StorageErr(StorageErr),
    #[error(transparent)]
    CacheErr(CacheErr),
}

impl From<StorageErr> for CrudErr {
    fn from(e: StorageErr) -> Self {
        Self::StorageErr(e)
    }
}

impl From<CacheErr> for CrudErr {
    fn from(e: CacheErr) -> Self {
        Self::CacheErr(e)
    }
}

crate::impl_error!(CrudErr {
    StorageErr,
    CacheErr,
});
