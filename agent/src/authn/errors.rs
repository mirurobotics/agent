use crate::crypt::errors::CryptErr;
use crate::errors::Trace;
use crate::filesys::errors::FileSysErr;
use crate::http::errors::HTTPErr;

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
#[error("mock error (is network connection error: {is_network_connection_error})")]
pub struct MockError {
    pub is_network_connection_error: bool,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for MockError {
    fn is_network_connection_error(&self) -> bool {
        self.is_network_connection_error
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuthnErr {
    #[error(transparent)]
    TimestampConversionErr(TimestampConversionErr),
    #[error(transparent)]
    CryptErr(CryptErr),
    #[error(transparent)]
    FileSysErr(FileSysErr),
    #[error(transparent)]
    HTTPErr(HTTPErr),
    #[error(transparent)]
    SerdeErr(SerdeErr),
    #[error(transparent)]
    SendActorMessageErr(SendActorMessageErr),
    #[error(transparent)]
    ReceiveActorMessageErr(ReceiveActorMessageErr),
    #[error(transparent)]
    MockError(MockError),
}

impl From<CryptErr> for AuthnErr {
    fn from(e: CryptErr) -> Self {
        Self::CryptErr(e)
    }
}

impl From<FileSysErr> for AuthnErr {
    fn from(e: FileSysErr) -> Self {
        Self::FileSysErr(e)
    }
}

impl From<HTTPErr> for AuthnErr {
    fn from(e: HTTPErr) -> Self {
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
