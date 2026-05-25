// internal crates
use crate::cache;
use crate::events;
use crate::filesys;
use crate::http;
use crate::models;
use crate::services::config_instance::get::ConfigInstanceNotFoundErr;
use crate::services::config_instance::parameter::{ContentParseErr, ParameterNotFoundErr};
use crate::storage::StorageErr;
use crate::sync;

#[derive(Debug, thiserror::Error)]
pub enum ServiceErr {
    #[error(transparent)]
    CacheErr(cache::CacheErr),
    #[error(transparent)]
    ConfigInstanceNotFoundErr(ConfigInstanceNotFoundErr),
    #[error(transparent)]
    ContentParseErr(ContentParseErr),
    #[error(transparent)]
    FileSysErr(filesys::FileSysErr),
    #[error(transparent)]
    ModelsErr(models::ModelsErr),
    #[error(transparent)]
    ParameterNotFoundErr(ParameterNotFoundErr),
    #[error(transparent)]
    StorageErr(StorageErr),
    #[error(transparent)]
    HTTPErr(http::HTTPErr),
    #[error(transparent)]
    EventsErr(events::errors::EventsErr),
    #[error(transparent)]
    SyncErr(sync::SyncErr),
}

impl From<cache::CacheErr> for ServiceErr {
    fn from(e: cache::CacheErr) -> Self {
        Self::CacheErr(e)
    }
}

impl From<ConfigInstanceNotFoundErr> for ServiceErr {
    fn from(e: ConfigInstanceNotFoundErr) -> Self {
        Self::ConfigInstanceNotFoundErr(e)
    }
}

impl From<ContentParseErr> for ServiceErr {
    fn from(e: ContentParseErr) -> Self {
        Self::ContentParseErr(e)
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

impl From<ParameterNotFoundErr> for ServiceErr {
    fn from(e: ParameterNotFoundErr) -> Self {
        Self::ParameterNotFoundErr(e)
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

crate::impl_error!(ServiceErr {
    CacheErr,
    ConfigInstanceNotFoundErr,
    ContentParseErr,
    EventsErr,
    FileSysErr,
    ModelsErr,
    ParameterNotFoundErr,
    StorageErr,
    HTTPErr,
    SyncErr,
});
