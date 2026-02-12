// standard library
use std::fmt;

// internal crates
use crate::cache::errors::CacheErr;
use crate::crud::errors::CrudErr;
use crate::errors::{Code, HTTPCode, MiruError, Trace};
use crate::filesys::errors::FileSysErr;
use crate::http::errors::HTTPErr;
use crate::models::errors::ModelsErr;
use crate::storage::errors::StorageErr;
use crate::sync::errors::SyncErr;

#[derive(Debug)]
pub struct ServiceModelsErr {
    pub source: ModelsErr,
    pub trace: Box<Trace>,
}

impl MiruError for ServiceModelsErr {
    fn code(&self) -> Code {
        self.source.code()
    }

    fn http_status(&self) -> HTTPCode {
        self.source.http_status()
    }

    fn is_network_connection_error(&self) -> bool {
        self.source.is_network_connection_error()
    }

    fn params(&self) -> Option<serde_json::Value> {
        self.source.params()
    }
}

impl fmt::Display for ServiceModelsErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Models Error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct ServiceSyncErr {
    pub source: SyncErr,
    pub trace: Box<Trace>,
}

impl MiruError for ServiceSyncErr {
    fn code(&self) -> Code {
        self.source.code()
    }

    fn http_status(&self) -> HTTPCode {
        self.source.http_status()
    }

    fn is_network_connection_error(&self) -> bool {
        self.source.is_network_connection_error()
    }

    fn params(&self) -> Option<serde_json::Value> {
        self.source.params()
    }
}

impl fmt::Display for ServiceSyncErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sync Error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct ServiceCacheErr {
    pub source: CacheErr,
    pub trace: Box<Trace>,
}

impl MiruError for ServiceCacheErr {
    fn code(&self) -> Code {
        self.source.code()
    }

    fn http_status(&self) -> HTTPCode {
        self.source.http_status()
    }

    fn is_network_connection_error(&self) -> bool {
        self.source.is_network_connection_error()
    }

    fn params(&self) -> Option<serde_json::Value> {
        self.source.params()
    }
}

impl fmt::Display for ServiceCacheErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Cache Error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct ServiceCrudErr {
    pub source: CrudErr,
    pub trace: Box<Trace>,
}

impl MiruError for ServiceCrudErr {
    fn code(&self) -> Code {
        self.source.code()
    }

    fn http_status(&self) -> HTTPCode {
        self.source.http_status()
    }

    fn is_network_connection_error(&self) -> bool {
        self.source.is_network_connection_error()
    }

    fn params(&self) -> Option<serde_json::Value> {
        self.source.params()
    }
}

impl fmt::Display for ServiceCrudErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Crud Error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct ServiceFileSysErr {
    pub source: FileSysErr,
    pub trace: Box<Trace>,
}

impl MiruError for ServiceFileSysErr {
    fn code(&self) -> Code {
        self.source.code()
    }

    fn http_status(&self) -> HTTPCode {
        self.source.http_status()
    }

    fn is_network_connection_error(&self) -> bool {
        self.source.is_network_connection_error()
    }

    fn params(&self) -> Option<serde_json::Value> {
        self.source.params()
    }
}

impl fmt::Display for ServiceFileSysErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FileSys Error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct ServiceStorageErr {
    pub source: StorageErr,
    pub trace: Box<Trace>,
}

impl MiruError for ServiceStorageErr {
    fn code(&self) -> Code {
        self.source.code()
    }

    fn http_status(&self) -> HTTPCode {
        self.source.http_status()
    }

    fn is_network_connection_error(&self) -> bool {
        self.source.is_network_connection_error()
    }

    fn params(&self) -> Option<serde_json::Value> {
        self.source.params()
    }
}

impl fmt::Display for ServiceStorageErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Storage Error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct ServiceHTTPErr {
    pub source: HTTPErr,
    pub trace: Box<Trace>,
}

impl MiruError for ServiceHTTPErr {
    fn code(&self) -> Code {
        self.source.code()
    }

    fn http_status(&self) -> HTTPCode {
        self.source.http_status()
    }

    fn is_network_connection_error(&self) -> bool {
        self.source.is_network_connection_error()
    }

    fn params(&self) -> Option<serde_json::Value> {
        self.source.params()
    }
}

impl fmt::Display for ServiceHTTPErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "service HTTP Error: {}", self.source)
    }
}

#[derive(Debug)]
pub enum ServiceErr {
    // internal crate errors
    CacheErr(Box<ServiceCacheErr>),
    CrudErr(Box<ServiceCrudErr>),
    FileSysErr(Box<ServiceFileSysErr>),
    ModelsErr(Box<ServiceModelsErr>),
    StorageErr(Box<ServiceStorageErr>),
    HTTPErr(Box<ServiceHTTPErr>),
    SyncErr(Box<ServiceSyncErr>),
}

macro_rules! forward_error_method {
    ($self:ident, $method:ident $(, $arg:expr)?) => {
        match $self {
            Self::CacheErr(e) => e.$method($($arg)?),
            Self::CrudErr(e) => e.$method($($arg)?),
            Self::FileSysErr(e) => e.$method($($arg)?),
            Self::ModelsErr(e) => e.$method($($arg)?),
            Self::StorageErr(e) => e.$method($($arg)?),
            Self::HTTPErr(e) => e.$method($($arg)?),
            Self::SyncErr(e) => e.$method($($arg)?),
        }
    };
}

impl fmt::Display for ServiceErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        forward_error_method!(self, fmt, f)
    }
}

impl MiruError for ServiceErr {
    fn code(&self) -> Code {
        forward_error_method!(self, code)
    }

    fn http_status(&self) -> HTTPCode {
        forward_error_method!(self, http_status)
    }

    fn is_network_connection_error(&self) -> bool {
        forward_error_method!(self, is_network_connection_error)
    }

    fn params(&self) -> Option<serde_json::Value> {
        forward_error_method!(self, params)
    }
}
