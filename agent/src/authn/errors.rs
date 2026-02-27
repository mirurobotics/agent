use crate::crypt;
use crate::errors::Trace;
use crate::filesys;
use crate::http;

#[derive(Debug, thiserror::Error)]
#[error("timestamp conversion error: {msg}")]
pub struct TimestampConversionErr {
    pub msg: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for TimestampConversionErr {}

#[derive(Debug, thiserror::Error)]
#[error("serialization error: {source}")]
pub struct SerdeErr {
    pub source: serde_json::Error,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for SerdeErr {}

#[derive(Debug, thiserror::Error)]
#[error("failed to send actor message: {source}")]
pub struct SendActorMessageErr {
    #[source]
    pub source: Box<dyn std::error::Error + Send + Sync>,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for SendActorMessageErr {}

#[derive(Debug, thiserror::Error)]
#[error("failed to receive actor message: {source}")]
pub struct ReceiveActorMessageErr {
    #[source]
    pub source: Box<dyn std::error::Error + Send + Sync>,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for ReceiveActorMessageErr {}

#[derive(Debug, thiserror::Error)]
#[error("mock error (is network connection error: {is_network_conn_err})")]
pub struct MockError {
    pub is_network_conn_err: bool,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for MockError {
    fn is_network_conn_err(&self) -> bool {
        self.is_network_conn_err
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuthnErr {
    #[error(transparent)]
    TimestampConversionErr(TimestampConversionErr),
    #[error(transparent)]
    CryptErr(crypt::CryptErr),
    #[error(transparent)]
    FileSysErr(filesys::FileSysErr),
    #[error(transparent)]
    HTTPErr(http::HTTPErr),
    #[error(transparent)]
    SerdeErr(SerdeErr),
    #[error(transparent)]
    SendActorMessageErr(SendActorMessageErr),
    #[error(transparent)]
    ReceiveActorMessageErr(ReceiveActorMessageErr),
    #[error(transparent)]
    MockError(MockError),
}

impl From<crypt::CryptErr> for AuthnErr {
    fn from(e: crypt::CryptErr) -> Self {
        Self::CryptErr(e)
    }
}

impl From<filesys::FileSysErr> for AuthnErr {
    fn from(e: filesys::FileSysErr) -> Self {
        Self::FileSysErr(e)
    }
}

impl From<http::HTTPErr> for AuthnErr {
    fn from(e: http::HTTPErr) -> Self {
        Self::HTTPErr(e)
    }
}

crate::impl_error!(AuthnErr {
    TimestampConversionErr,
    CryptErr,
    FileSysErr,
    HTTPErr,
    SerdeErr,
    SendActorMessageErr,
    ReceiveActorMessageErr,
    MockError,
});
