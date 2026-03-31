// internal crates
use crate::errors::Trace;

pub type SendActorMessageErr = crate::cache::errors::SendActorMessageErr;
pub type ReceiveActorMessageErr = crate::cache::errors::ReceiveActorMessageErr;

#[derive(Debug, thiserror::Error)]
#[error("event store I/O error: {0}")]
pub struct IoErr(pub std::io::Error);
impl crate::errors::Error for IoErr {}

#[derive(Debug, thiserror::Error)]
#[error("event serialization error: {0}")]
pub struct SerializationErr(pub serde_json::Error);
impl crate::errors::Error for SerializationErr {}

#[derive(Debug, thiserror::Error)]
#[error("cursor {requested} has expired; earliest available event is {earliest_available}")]
pub struct CursorExpiredErr {
    pub earliest_available: u64,
    pub requested: u64,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for CursorExpiredErr {
    fn code(&self) -> crate::errors::Code {
        crate::errors::Code::CursorExpired
    }
    fn http_status(&self) -> crate::errors::HTTPCode {
        crate::errors::HTTPCode::GONE
    }
}

#[derive(Debug, thiserror::Error)]
#[error("malformed cursor value")]
pub struct MalformedCursorErr {
    pub trace: Box<Trace>,
}

impl crate::errors::Error for MalformedCursorErr {
    fn code(&self) -> crate::errors::Code {
        crate::errors::Code::MalformedCursor
    }
    fn http_status(&self) -> crate::errors::HTTPCode {
        crate::errors::HTTPCode::BAD_REQUEST
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EventsErr {
    #[error(transparent)]
    IoErr(IoErr),
    #[error(transparent)]
    FileSysErr(#[from] crate::filesys::FileSysErr),
    #[error(transparent)]
    SerializationErr(SerializationErr),
    #[error(transparent)]
    CursorExpiredErr(CursorExpiredErr),
    #[error(transparent)]
    MalformedCursorErr(MalformedCursorErr),
    #[error(transparent)]
    SendActorMessageErr(SendActorMessageErr),
    #[error(transparent)]
    ReceiveActorMessageErr(ReceiveActorMessageErr),
}

impl From<std::io::Error> for EventsErr {
    fn from(e: std::io::Error) -> Self {
        Self::IoErr(IoErr(e))
    }
}

impl From<serde_json::Error> for EventsErr {
    fn from(e: serde_json::Error) -> Self {
        Self::SerializationErr(SerializationErr(e))
    }
}

crate::impl_error!(EventsErr {
    IoErr,
    FileSysErr,
    SerializationErr,
    CursorExpiredErr,
    MalformedCursorErr,
    SendActorMessageErr,
    ReceiveActorMessageErr,
});
