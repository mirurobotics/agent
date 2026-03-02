use crate::authn;
use crate::cache;
use crate::crypt;
use crate::errors::Trace;
use crate::events;
use crate::filesys;
use crate::http;
use crate::services;
use crate::storage::StorageErr;
use crate::sync;

#[derive(Debug, thiserror::Error)]
pub struct MissingDeviceIDErr {
    pub device_file_err: filesys::FileSysErr,
    pub jwt_err: crypt::CryptErr,
    pub trace: Box<Trace>,
}

impl std::fmt::Display for MissingDeviceIDErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unable to determine device id from the device file or the token on file: device file error: {}, jwt error: {}", self.device_file_err, self.jwt_err)
    }
}

impl crate::errors::Error for MissingDeviceIDErr {}

#[derive(Debug, thiserror::Error)]
#[error("shutdown manager was provided the same argument ({arg_name}) twice")]
pub struct ShutdownMngrDuplicateArgErr {
    pub arg_name: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for ShutdownMngrDuplicateArgErr {}

#[derive(Debug, thiserror::Error)]
#[error("failed to bind unix socket '{socket_file}': {source}")]
pub struct BindUnixSocketErr {
    pub socket_file: filesys::File,
    pub source: std::io::Error,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for BindUnixSocketErr {}

#[derive(Debug, thiserror::Error)]
#[error("failed to run axum server: {source}")]
pub struct RunAxumServerErr {
    pub source: std::io::Error,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for RunAxumServerErr {}

#[derive(Debug, thiserror::Error)]
#[error("join handle error: {source}")]
pub struct JoinHandleErr {
    #[source]
    pub source: Box<dyn std::error::Error + Send + Sync>,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for JoinHandleErr {}

#[derive(Debug, thiserror::Error)]
#[error("failed to send shutdown signal to {service}")]
pub struct SendShutdownSignalErr {
    pub service: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for SendShutdownSignalErr {}

#[derive(Debug, thiserror::Error)]
#[error("timestamp conversion error: {msg}")]
pub struct TimestampConversionErr {
    pub msg: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for TimestampConversionErr {}

#[derive(Debug, thiserror::Error)]
#[error("failed to initialize event hub: {source}")]
pub struct EventHubInitErr {
    #[source]
    pub source: events::EventErr,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for EventHubInitErr {}

#[derive(Debug, thiserror::Error)]
pub enum ServerErr {
    // server errors
    #[error(transparent)]
    MissingDeviceIDErr(Box<MissingDeviceIDErr>),
    #[error(transparent)]
    TimestampConversionErr(TimestampConversionErr),
    #[error(transparent)]
    ShutdownMngrDuplicateArgErr(ShutdownMngrDuplicateArgErr),

    // internal crate errors
    #[error(transparent)]
    AuthnErr(authn::AuthnErr),
    #[error(transparent)]
    CacheErr(cache::CacheErr),
    #[error(transparent)]
    CryptErr(crypt::CryptErr),
    #[error(transparent)]
    FileSysErr(filesys::FileSysErr),
    #[error(transparent)]
    HTTPErr(http::HTTPErr),
    #[error(transparent)]
    ServiceErr(services::ServiceErr),
    #[error(transparent)]
    StorageErr(StorageErr),
    #[error(transparent)]
    SyncErr(Box<sync::SyncErr>),

    // event hub errors
    #[error(transparent)]
    EventHubInitErr(EventHubInitErr),

    // external crate errors
    #[error(transparent)]
    BindUnixSocketErr(BindUnixSocketErr),
    #[error(transparent)]
    RunAxumServerErr(RunAxumServerErr),
    #[error(transparent)]
    SendShutdownSignalErr(SendShutdownSignalErr),
    #[error(transparent)]
    JoinHandleErr(JoinHandleErr),
}

impl From<authn::AuthnErr> for ServerErr {
    fn from(e: authn::AuthnErr) -> Self {
        Self::AuthnErr(e)
    }
}

impl From<cache::CacheErr> for ServerErr {
    fn from(e: cache::CacheErr) -> Self {
        Self::CacheErr(e)
    }
}

impl From<crypt::CryptErr> for ServerErr {
    fn from(e: crypt::CryptErr) -> Self {
        Self::CryptErr(e)
    }
}

impl From<filesys::FileSysErr> for ServerErr {
    fn from(e: filesys::FileSysErr) -> Self {
        Self::FileSysErr(e)
    }
}

impl From<http::HTTPErr> for ServerErr {
    fn from(e: http::HTTPErr) -> Self {
        Self::HTTPErr(e)
    }
}

impl From<services::ServiceErr> for ServerErr {
    fn from(e: services::ServiceErr) -> Self {
        Self::ServiceErr(e)
    }
}

impl From<StorageErr> for ServerErr {
    fn from(e: StorageErr) -> Self {
        Self::StorageErr(e)
    }
}

impl From<sync::SyncErr> for ServerErr {
    fn from(e: sync::SyncErr) -> Self {
        Self::SyncErr(Box::new(e))
    }
}

crate::impl_error!(ServerErr {
    MissingDeviceIDErr,
    TimestampConversionErr,
    ShutdownMngrDuplicateArgErr,
    AuthnErr,
    CacheErr,
    CryptErr,
    EventHubInitErr,
    FileSysErr,
    HTTPErr,
    ServiceErr,
    StorageErr,
    SyncErr,
    BindUnixSocketErr,
    RunAxumServerErr,
    SendShutdownSignalErr,
    JoinHandleErr,
});
