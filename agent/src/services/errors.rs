// internal crates
use crate::cache;
use crate::disk::DiskErr;
use crate::events;
use crate::filesys;
use crate::http;
use crate::models;
use crate::sync;

#[derive(Debug, thiserror::Error)]
#[error(
    "release '{release_id}' did not have file_rules expansion (backend did not expand file_rules)"
)]
pub struct FileRulesNotExpandedErr {
    pub release_id: String,
}

impl crate::errors::Error for FileRulesNotExpandedErr {}

#[derive(Debug, thiserror::Error)]
pub enum ServiceErr {
    #[error(transparent)]
    CacheErr(cache::CacheErr),
    #[error(transparent)]
    FileSysErr(filesys::FileSysErr),
    #[error(transparent)]
    ModelsErr(models::ModelsErr),
    #[error(transparent)]
    DiskErr(DiskErr),
    #[error(transparent)]
    HTTPErr(http::HTTPErr),
    #[error(transparent)]
    EventsErr(events::errors::EventsErr),
    #[error(transparent)]
    SyncErr(sync::SyncErr),
    #[error(transparent)]
    FileRulesNotExpanded(FileRulesNotExpandedErr),
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

impl From<DiskErr> for ServiceErr {
    fn from(e: DiskErr) -> Self {
        Self::DiskErr(e)
    }
}

impl From<http::HTTPErr> for ServiceErr {
    fn from(e: http::HTTPErr) -> Self {
        Self::HTTPErr(e)
    }
}

impl From<events::errors::EventsErr> for ServiceErr {
    fn from(e: events::errors::EventsErr) -> Self {
        Self::EventsErr(e)
    }
}

impl From<sync::SyncErr> for ServiceErr {
    fn from(e: sync::SyncErr) -> Self {
        Self::SyncErr(e)
    }
}

impl From<FileRulesNotExpandedErr> for ServiceErr {
    fn from(e: FileRulesNotExpandedErr) -> Self {
        Self::FileRulesNotExpanded(e)
    }
}

crate::impl_error!(ServiceErr {
    CacheErr,
    EventsErr,
    FileSysErr,
    ModelsErr,
    DiskErr,
    HTTPErr,
    SyncErr,
    FileRulesNotExpanded,
});
