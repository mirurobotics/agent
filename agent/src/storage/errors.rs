// standard library
use std::fmt;

// internal crates
use crate::cache::errors::CacheErr;
use crate::crypt::errors::CryptErr;
use crate::errors::{Code, HTTPCode, Error, Trace};
use crate::filesys::errors::FileSysErr;

#[derive(Debug)]
pub struct DeviceNotActivatedErr {
    pub msg: String,
    pub trace: Box<Trace>,
}

impl Error for DeviceNotActivatedErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for DeviceNotActivatedErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "device is not activated: {}", self.msg)
    }
}

#[derive(Debug)]
pub struct StorageCacheErr {
    pub source: CacheErr,
    pub trace: Box<Trace>,
}

impl Error for StorageCacheErr {
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

impl fmt::Display for StorageCacheErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cache error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct StorageCryptErr {
    pub source: CryptErr,
    pub trace: Box<Trace>,
}

impl Error for StorageCryptErr {
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

impl fmt::Display for StorageCryptErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "crypt error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct StorageFileSysErr {
    pub source: FileSysErr,
    pub trace: Box<Trace>,
}

impl Error for StorageFileSysErr {
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

impl fmt::Display for StorageFileSysErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "file system error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct JoinHandleErr {
    pub source: Box<dyn std::error::Error + Send + Sync>,
    pub trace: Box<Trace>,
}

impl Error for JoinHandleErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for JoinHandleErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "failed to join handle: {}", self.source)
    }
}

#[derive(Debug)]
pub struct PruneCacheErrs {
    pub sources: Vec<CacheErr>,
    pub trace: Box<Trace>,
}

impl Error for PruneCacheErrs {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        false
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for PruneCacheErrs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "failed to prune caches: {:?}", self.sources)
    }
}

#[derive(Debug)]
pub enum StorageErr {
    // storage errors
    DeviceNotActivatedErr(Box<DeviceNotActivatedErr>),
    PruneCacheErrs(Box<PruneCacheErrs>),

    // internal crate errors
    CacheErr(Box<StorageCacheErr>),
    CryptErr(Box<StorageCryptErr>),
    FileSysErr(Box<StorageFileSysErr>),

    // external crate errors
    JoinHandleErr(Box<JoinHandleErr>),
}

macro_rules! forward_error_method {
    ($self:ident, $method:ident $(, $arg:expr)?) => {
        match $self {
            Self::DeviceNotActivatedErr(e) => e.$method($($arg)?),
            Self::PruneCacheErrs(e) => e.$method($($arg)?),
            Self::CacheErr(e) => e.$method($($arg)?),
            Self::CryptErr(e) => e.$method($($arg)?),
            Self::FileSysErr(e) => e.$method($($arg)?),
            Self::JoinHandleErr(e) => e.$method($($arg)?),
        }
    };
}

impl fmt::Display for StorageErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        forward_error_method!(self, fmt, f)
    }
}

impl Error for StorageErr {
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
