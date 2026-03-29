// internal crates
use crate::errors::Trace;

pub type SendActorMessageErr = crate::cache::errors::SendActorMessageErr;
pub type ReceiveActorMessageErr = crate::cache::errors::ReceiveActorMessageErr;

#[derive(Debug, thiserror::Error)]
#[error("cursor {requested} has expired; earliest available event is {earliest_available}")]
pub struct CursorExpiredErr {
    pub earliest_available: u64,
    pub requested: u64,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for CursorExpiredErr {
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
    fn http_status(&self) -> crate::errors::HTTPCode {
        crate::errors::HTTPCode::BAD_REQUEST
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EventsErr {
    #[error("event store I/O error: {0}")]
    IoErr(#[from] std::io::Error),
    #[error("event store filesystem error: {0}")]
    FileSysErr(#[from] crate::filesys::FileSysErr),
    #[error("event serialization error: {0}")]
    SerializationErr(#[from] serde_json::Error),
    #[error(transparent)]
    CursorExpiredErr(CursorExpiredErr),
    #[error(transparent)]
    MalformedCursorErr(MalformedCursorErr),
    #[error(transparent)]
    SendActorMessageErr(SendActorMessageErr),
    #[error(transparent)]
    ReceiveActorMessageErr(ReceiveActorMessageErr),
}

impl crate::errors::Error for EventsErr {
    fn http_status(&self) -> crate::errors::HTTPCode {
        match self {
            Self::CursorExpiredErr(e) => e.http_status(),
            Self::MalformedCursorErr(e) => e.http_status(),
            _ => crate::errors::HTTPCode::INTERNAL_SERVER_ERROR,
        }
    }
}
