// standard library
use std::fmt;

// internal crates
use crate::cache::errors::CacheErr;
use crate::errors::{Code, HTTPCode, Error, Trace};
use crate::storage::errors::StorageErr;

// external crates
use serde_json;

#[derive(Debug)]
pub struct CrudStorageErr {
    pub source: StorageErr,
    pub trace: Box<Trace>,
}

impl Error for CrudStorageErr {
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

impl fmt::Display for CrudStorageErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.source)
    }
}

#[derive(Debug)]
pub struct CrudCacheErr {
    pub source: CacheErr,
    pub trace: Box<Trace>,
}

impl Error for CrudCacheErr {
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

impl fmt::Display for CrudCacheErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.source)
    }
}

#[derive(Debug)]
pub enum CrudErr {
    StorageErr(Box<CrudStorageErr>),
    CacheErr(Box<CrudCacheErr>),
}

macro_rules! forward_error_method {
    ($self:ident, $method:ident $(, $arg:expr)?) => {
        match $self {
            CrudErr::StorageErr(e) => e.$method($($arg)?),
            CrudErr::CacheErr(e) => e.$method($($arg)?),
        }
    };
}

impl fmt::Display for CrudErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        forward_error_method!(self, fmt, f)
    }
}

impl Error for CrudErr {
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
