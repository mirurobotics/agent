// standard crates
use std::fmt;

// internal crates
use crate::authn::errors::AuthnErr;
use crate::cache::errors::CacheErr;
use crate::crud::errors::CrudErr;
use crate::deploy::errors::DeployErr;
use crate::errors::{Code, HTTPCode, Error, Trace};
use crate::filesys::errors::FileSysErr;
use crate::http::errors::HTTPErr;
use crate::storage::errors::StorageErr;

// external crates
use chrono::{DateTime, Utc};

#[derive(Debug)]
pub struct SyncAuthnErr {
    pub source: AuthnErr,
    pub trace: Box<Trace>,
}

impl Error for SyncAuthnErr {
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

impl fmt::Display for SyncAuthnErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Auth error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct SyncCacheErr {
    pub source: CacheErr,
    pub trace: Box<Trace>,
}

impl Error for SyncCacheErr {
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

impl fmt::Display for SyncCacheErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Cache error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct SyncCrudErr {
    pub source: CrudErr,
    pub trace: Box<Trace>,
}

impl Error for SyncCrudErr {
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

impl fmt::Display for SyncCrudErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Crud error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct SyncDeployErr {
    pub source: DeployErr,
    pub trace: Box<Trace>,
}

impl Error for SyncDeployErr {
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

impl fmt::Display for SyncDeployErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Deploy error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct SyncHTTPClientErr {
    pub source: HTTPErr,
    pub trace: Box<Trace>,
}

impl Error for SyncHTTPClientErr {
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

impl fmt::Display for SyncHTTPClientErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HTTP client error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct SyncFileSysErr {
    pub source: FileSysErr,
    pub trace: Box<Trace>,
}

impl Error for SyncFileSysErr {
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

impl fmt::Display for SyncFileSysErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "file system error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct SyncStorageErr {
    pub source: StorageErr,
    pub trace: Box<Trace>,
}

impl Error for SyncStorageErr {
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

impl fmt::Display for SyncStorageErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Storage error: {}", self.source)
    }
}

#[derive(Debug)]
pub struct MissingExpandedInstancesErr {
    pub expected_ids: Vec<String>,
    pub actual_ids: Vec<String>,
    pub trace: Box<Trace>,
}

impl Error for MissingExpandedInstancesErr {
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

impl fmt::Display for MissingExpandedInstancesErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Missing expanded config instances: expected ids: {:?}, actual ids: {:?}",
            self.expected_ids, self.actual_ids
        )
    }
}

#[derive(Debug)]
pub struct SyncErrors {
    pub source: Vec<SyncErr>,
    pub trace: Box<Trace>,
}

impl Error for SyncErrors {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        // is only a network connection error if all errors are network connection
        // errors
        for err in self.source.iter() {
            if !err.is_network_connection_error() {
                return false;
            }
        }
        true
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for SyncErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sync error: {:?}", self.source)
    }
}

#[derive(Debug)]
pub struct ConfigInstanceContentNotFoundErr {
    pub cfg_inst_id: String,
    pub trace: Box<Trace>,
}

impl Error for ConfigInstanceContentNotFoundErr {
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

impl fmt::Display for ConfigInstanceContentNotFoundErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Config instance content not found for config instance '{}'",
            self.cfg_inst_id
        )
    }
}

pub type SendActorMessageErr = crate::cache::errors::SendActorMessageErr;
pub type ReceiveActorMessageErr = crate::cache::errors::ReceiveActorMessageErr;

#[derive(Debug)]
pub struct SyncerInCooldownErr {
    pub err_streak: u32,
    pub cooldown_ends_at: DateTime<Utc>,
    pub trace: Box<Trace>,
}

impl Error for SyncerInCooldownErr {
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

impl fmt::Display for SyncerInCooldownErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let err_streak = self.err_streak;
        let cooldown_secs = self
            .cooldown_ends_at
            .signed_duration_since(Utc::now())
            .num_seconds();
        let cooldown_ends_at = self.cooldown_ends_at;
        write!(f, "cannot sync device because the syncer is in cooldown (err streak of {err_streak}) for {cooldown_secs} seconds (cooldown ends at: {cooldown_ends_at})",
        )
    }
}

#[derive(Debug)]
pub struct MockErr {
    pub is_network_connection_error: bool,
}

impl Error for MockErr {
    fn code(&self) -> Code {
        Code::InternalServerError
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INTERNAL_SERVER_ERROR
    }

    fn is_network_connection_error(&self) -> bool {
        self.is_network_connection_error
    }

    fn params(&self) -> Option<serde_json::Value> {
        None
    }
}

impl fmt::Display for MockErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Mock error")
    }
}

#[derive(Debug)]
pub enum SyncErr {
    AuthnErr(Box<SyncAuthnErr>),
    CacheErr(Box<SyncCacheErr>),
    CrudErr(Box<SyncCrudErr>),
    DeployErr(Box<SyncDeployErr>),
    FileSysErr(Box<SyncFileSysErr>),
    HTTPClientErr(Box<SyncHTTPClientErr>),
    StorageErr(Box<SyncStorageErr>),
    SyncErrors(Box<SyncErrors>),

    MissingExpandedInstancesErr(Box<MissingExpandedInstancesErr>),
    InCooldownErr(Box<SyncerInCooldownErr>),
    ConfigInstanceContentNotFound(Box<ConfigInstanceContentNotFoundErr>),
    SendActorMessageErr(Box<SendActorMessageErr>),
    ReceiveActorMessageErr(Box<ReceiveActorMessageErr>),

    MockErr(Box<MockErr>),
}

macro_rules! forward_error_method {
    ($self:ident, $method:ident $(, $arg:expr)?) => {
        match $self {
            SyncErr::AuthnErr(e) => e.$method($($arg)?),
            SyncErr::CacheErr(e) => e.$method($($arg)?),
            SyncErr::CrudErr(e) => e.$method($($arg)?),
            SyncErr::DeployErr(e) => e.$method($($arg)?),
            SyncErr::FileSysErr(e) => e.$method($($arg)?),
            SyncErr::HTTPClientErr(e) => e.$method($($arg)?),
            SyncErr::StorageErr(e) => e.$method($($arg)?),
            SyncErr::SyncErrors(e) => e.$method($($arg)?),

            SyncErr::MissingExpandedInstancesErr(e) => e.$method($($arg)?),
            SyncErr::InCooldownErr(e) => e.$method($($arg)?),
            SyncErr::ConfigInstanceContentNotFound(e) => e.$method($($arg)?),
            SyncErr::SendActorMessageErr(e) => e.$method($($arg)?),
            SyncErr::ReceiveActorMessageErr(e) => e.$method($($arg)?),

            SyncErr::MockErr(e) => e.$method($($arg)?),
        }
    };
}

impl fmt::Display for SyncErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        forward_error_method!(self, fmt, f)
    }
}

impl Error for SyncErr {
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
