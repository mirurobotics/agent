// standard library
use std::fmt;

// internal crates
use crate::errors::{Code, HTTPCode, Error, Trace};
use crate::filesys::errors::FileSysErr;

#[derive(Debug)]
pub struct CacheElementNotFound {
    pub msg: String,
    pub trace: Box<Trace>,
}

impl Error for CacheElementNotFound {
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

impl fmt::Display for CacheElementNotFound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unable to find cache element: {}", self.msg)
    }
}

#[derive(Debug)]
pub struct CacheFileSysErr {
    pub source: FileSysErr,
    pub trace: Box<Trace>,
}

impl Error for CacheFileSysErr {
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

impl fmt::Display for CacheFileSysErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "file system error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct FoundTooManyCacheElements {
    pub expected_count: usize,
    pub actual_count: usize,
    pub filter_name: String,
    pub trace: Box<Trace>,
}

impl Error for FoundTooManyCacheElements {
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

impl fmt::Display for FoundTooManyCacheElements {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "expected to find {} elements when filtering by '{}' but found {}",
            self.expected_count, self.filter_name, self.actual_count
        )
    }
}

#[derive(Debug)]
pub struct CannotOverwriteCacheElement {
    pub key: String,
    pub trace: Box<Trace>,
}

impl Error for CannotOverwriteCacheElement {
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

impl fmt::Display for CannotOverwriteCacheElement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot overwrite cache element: {}", self.key)
    }
}

#[derive(Debug)]
pub struct SendActorMessageErr {
    pub source: Box<dyn std::error::Error + Send + Sync>,
    pub trace: Box<Trace>,
}

impl Error for SendActorMessageErr {
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

impl fmt::Display for SendActorMessageErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "failed to send actor message: {:?}", self.source)
    }
}

#[derive(Debug)]
pub struct ReceiveActorMessageErr {
    pub source: Box<dyn std::error::Error + Send + Sync>,
    pub trace: Box<Trace>,
}

impl Error for ReceiveActorMessageErr {
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

impl fmt::Display for ReceiveActorMessageErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "failed to receive actor message: {}", self.source)
    }
}

#[derive(Debug)]
pub enum CacheErr {
    CacheElementNotFound(Box<CacheElementNotFound>),
    CannotOverwriteCacheElement(Box<CannotOverwriteCacheElement>),
    FileSysErr(Box<CacheFileSysErr>),
    FoundTooManyCacheElements(Box<FoundTooManyCacheElements>),
    SendActorMessageErr(Box<SendActorMessageErr>),
    ReceiveActorMessageErr(Box<ReceiveActorMessageErr>),
}

macro_rules! forward_error_method {
    ($self:ident, $method:ident $(, $arg:expr)?) => {
        match $self {
            Self::CacheElementNotFound(e) => e.$method($($arg)?),
            Self::CannotOverwriteCacheElement(e) => e.$method($($arg)?),
            Self::FileSysErr(e) => e.$method($($arg)?),
            Self::FoundTooManyCacheElements(e) => e.$method($($arg)?),
            Self::SendActorMessageErr(e) => e.$method($($arg)?),
            Self::ReceiveActorMessageErr(e) => e.$method($($arg)?),
        }
    };
}

impl fmt::Display for CacheErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        forward_error_method!(self, fmt, f)
    }
}

impl Error for CacheErr {
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
