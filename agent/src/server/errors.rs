// standard library
use std::fmt;

// internal crates
use crate::authn::errors::AuthnErr;
use crate::cache::errors::CacheErr;
use crate::crypt::errors::CryptErr;
use crate::errors::Error;
use crate::errors::{Code, HTTPCode, Trace};
use crate::filesys::errors::FileSysErr;
use crate::filesys::file::File;
use crate::http::errors::HTTPErr;
use crate::services::errors::ServiceErr;
use crate::storage::errors::StorageErr;
use crate::sync::errors::SyncErr;

#[derive(Debug)]
pub struct ServerCacheErr {
    pub source: CacheErr,
    pub trace: Box<Trace>,
}

impl Error for ServerCacheErr {
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

impl fmt::Display for ServerCacheErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "server cache error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct MissingDeviceIDErr {
    pub device_file_err: FileSysErr,
    pub jwt_err: CryptErr,
    pub trace: Box<Trace>,
}

impl Error for MissingDeviceIDErr {
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

impl fmt::Display for MissingDeviceIDErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unable to determine device id from the device file or the token on file: device file error: {}, jwt error: {}", self.device_file_err, self.jwt_err)
    }
}

#[derive(Debug)]
pub struct ShutdownMngrDuplicateArgErr {
    pub arg_name: String,
    pub trace: Box<Trace>,
}

impl Error for ShutdownMngrDuplicateArgErr {
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

impl fmt::Display for ShutdownMngrDuplicateArgErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "shutdown manager was provided the same argument ({}) twice",
            self.arg_name
        )
    }
}

#[derive(Debug)]
pub struct ServerAuthnErr {
    pub source: AuthnErr,
    pub trace: Box<Trace>,
}

impl Error for ServerAuthnErr {
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

impl fmt::Display for ServerAuthnErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "server auth error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct ServerCryptErr {
    pub source: CryptErr,
    pub trace: Box<Trace>,
}

impl Error for ServerCryptErr {
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

impl fmt::Display for ServerCryptErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "server crypt error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct ServerFileSysErr {
    pub source: FileSysErr,
    pub trace: Box<Trace>,
}

impl Error for ServerFileSysErr {
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

impl fmt::Display for ServerFileSysErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "server file system error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct ServerHTTPErr {
    pub source: HTTPErr,
    pub trace: Box<Trace>,
}

impl Error for ServerHTTPErr {
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

impl fmt::Display for ServerHTTPErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "http client error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct ServerStorageErr {
    pub source: StorageErr,
    pub trace: Box<Trace>,
}

impl Error for ServerStorageErr {
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

impl fmt::Display for ServerStorageErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "server storage error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct ServerServiceErr {
    pub source: ServiceErr,
    pub trace: Box<Trace>,
}

impl Error for ServerServiceErr {
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

impl fmt::Display for ServerServiceErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "server service error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct ServerSyncErr {
    pub source: SyncErr,
    pub trace: Box<Trace>,
}

impl Error for ServerSyncErr {
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

impl fmt::Display for ServerSyncErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "server sync error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct BindUnixSocketErr {
    pub socket_file: File,
    pub source: std::io::Error,
    pub trace: Box<Trace>,
}

impl Error for BindUnixSocketErr {
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

impl fmt::Display for BindUnixSocketErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "failed to bind unix socket '{}': {}",
            self.socket_file, self.source
        )
    }
}

#[derive(Debug)]
pub struct RunAxumServerErr {
    pub source: std::io::Error,
    pub trace: Box<Trace>,
}

impl Error for RunAxumServerErr {
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

impl fmt::Display for RunAxumServerErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "failed to run axum server: {}", self.source)
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
        write!(f, "join handle error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct SendShutdownSignalErr {
    pub service: String,
    pub trace: Box<Trace>,
}

impl Error for SendShutdownSignalErr {
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

impl fmt::Display for SendShutdownSignalErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "failed to send shutdown signal to {}", self.service)
    }
}

#[derive(Debug)]
pub struct TimestampConversionErr {
    pub msg: String,
    pub trace: Box<Trace>,
}

impl Error for TimestampConversionErr {
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

impl fmt::Display for TimestampConversionErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "timestamp conversion error: {}", self.msg)
    }
}

#[derive(Debug)]
pub enum ServerErr {
    // server errors
    MissingDeviceIDErr(Box<MissingDeviceIDErr>),
    TimestampConversionErr(Box<TimestampConversionErr>),
    ShutdownMngrDuplicateArgErr(Box<ShutdownMngrDuplicateArgErr>),

    // internal crate errors
    AuthnErr(Box<ServerAuthnErr>),
    CacheErr(Box<ServerCacheErr>),
    CryptErr(Box<ServerCryptErr>),
    FileSysErr(Box<ServerFileSysErr>),
    HTTPErr(Box<ServerHTTPErr>),
    ServiceErr(Box<ServerServiceErr>),
    StorageErr(Box<ServerStorageErr>),
    SyncErr(Box<ServerSyncErr>),

    // external crate errors
    BindUnixSocketErr(Box<BindUnixSocketErr>),
    RunAxumServerErr(Box<RunAxumServerErr>),
    SendShutdownSignalErr(Box<SendShutdownSignalErr>),
    JoinHandleErr(Box<JoinHandleErr>),
}

macro_rules! forward_error_method {
    ($self:ident, $method:ident $(, $arg:expr)?) => {
        match $self {
            Self::MissingDeviceIDErr(e) => e.$method($($arg)?),
            Self::TimestampConversionErr(e) => e.$method($($arg)?),
            Self::ShutdownMngrDuplicateArgErr(e) => e.$method($($arg)?),
            Self::AuthnErr(e) => e.$method($($arg)?),
            Self::CryptErr(e) => e.$method($($arg)?),
            Self::FileSysErr(e) => e.$method($($arg)?),
            Self::HTTPErr(e) => e.$method($($arg)?),
            Self::ServiceErr(e) => e.$method($($arg)?),
            Self::StorageErr(e) => e.$method($($arg)?),
            Self::SyncErr(e) => e.$method($($arg)?),
            Self::BindUnixSocketErr(e) => e.$method($($arg)?),
            Self::RunAxumServerErr(e) => e.$method($($arg)?),
            Self::SendShutdownSignalErr(e) => e.$method($($arg)?),
            Self::JoinHandleErr(e) => e.$method($($arg)?),
            Self::CacheErr(e) => e.$method($($arg)?),
        }
    };
}

impl fmt::Display for ServerErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        forward_error_method!(self, fmt, f)
    }
}

impl Error for ServerErr {
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
