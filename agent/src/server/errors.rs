use crate::authn::errors::AuthnErr;
use crate::cache::errors::CacheErr;
use crate::crypt::errors::CryptErr;
use crate::errors::Trace;
use crate::filesys::errors::FileSysErr;
use crate::filesys::file::File;
use crate::http::errors::HTTPErr;
use crate::services::errors::ServiceErr;
use crate::storage::errors::StorageErr;
use crate::sync::errors::SyncErr;

#[derive(Debug, thiserror::Error)]
pub struct MissingDeviceIDErr {
    pub device_file_err: FileSysErr,
    pub jwt_err: CryptErr,
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
    pub socket_file: File,
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
    AuthnErr(AuthnErr),
    #[error(transparent)]
    CacheErr(CacheErr),
    #[error(transparent)]
    CryptErr(CryptErr),
    #[error(transparent)]
    FileSysErr(FileSysErr),
    #[error(transparent)]
    HTTPErr(HTTPErr),
    #[error(transparent)]
    ServiceErr(ServiceErr),
    #[error(transparent)]
    StorageErr(StorageErr),
    #[error(transparent)]
    SyncErr(Box<SyncErr>),

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

impl From<AuthnErr> for ServerErr {
    fn from(e: AuthnErr) -> Self {
        Self::AuthnErr(e)
    }
}

impl From<CacheErr> for ServerErr {
    fn from(e: CacheErr) -> Self {
        Self::CacheErr(e)
    }
}

impl From<CryptErr> for ServerErr {
    fn from(e: CryptErr) -> Self {
        Self::CryptErr(e)
    }
}

impl From<FileSysErr> for ServerErr {
    fn from(e: FileSysErr) -> Self {
        Self::FileSysErr(e)
    }
}

impl From<HTTPErr> for ServerErr {
    fn from(e: HTTPErr) -> Self {
        Self::HTTPErr(e)
    }
}

impl From<ServiceErr> for ServerErr {
    fn from(e: ServiceErr) -> Self {
        Self::ServiceErr(e)
    }
}

impl From<StorageErr> for ServerErr {
    fn from(e: StorageErr) -> Self {
        Self::StorageErr(e)
    }
}

impl From<SyncErr> for ServerErr {
    fn from(e: SyncErr) -> Self {
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
