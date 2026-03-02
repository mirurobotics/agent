// external crates
use axum::http::StatusCode;

#[derive(Debug, thiserror::Error)]
#[error("failed to initialize event store: {source}")]
pub struct InitErr {
    #[source]
    pub source: std::io::Error,
}

impl crate::errors::Error for InitErr {}

#[derive(Debug, thiserror::Error)]
#[error("failed to append event: {source}")]
pub struct AppendErr {
    #[source]
    pub source: std::io::Error,
}

impl crate::errors::Error for AppendErr {}

#[derive(Debug, thiserror::Error)]
#[error("failed to read event log: {source}")]
pub struct ReadErr {
    #[source]
    pub source: std::io::Error,
}

impl crate::errors::Error for ReadErr {}

#[derive(Debug, thiserror::Error)]
#[error("malformed replay cursor: {value}")]
pub struct MalformedCursorErr {
    pub value: String,
}

impl crate::errors::Error for MalformedCursorErr {
    fn http_status(&self) -> crate::errors::HTTPCode {
        StatusCode::BAD_REQUEST
    }

    fn code(&self) -> crate::errors::Code {
        crate::errors::Code::BackendError("malformed_cursor".to_string())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("cursor {cursor} is older than earliest retained event {earliest}")]
pub struct CursorExpiredErr {
    pub cursor: u64,
    pub earliest: u64,
}

impl crate::errors::Error for CursorExpiredErr {
    fn http_status(&self) -> crate::errors::HTTPCode {
        StatusCode::GONE
    }

    fn code(&self) -> crate::errors::Code {
        crate::errors::Code::BackendError("cursor_expired".to_string())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EventErr {
    #[error(transparent)]
    InitErr(InitErr),
    #[error(transparent)]
    AppendErr(AppendErr),
    #[error(transparent)]
    ReadErr(ReadErr),
    #[error(transparent)]
    MalformedCursorErr(MalformedCursorErr),
    #[error(transparent)]
    CursorExpiredErr(CursorExpiredErr),
}

impl From<InitErr> for EventErr {
    fn from(e: InitErr) -> Self {
        Self::InitErr(e)
    }
}

impl From<AppendErr> for EventErr {
    fn from(e: AppendErr) -> Self {
        Self::AppendErr(e)
    }
}

impl From<ReadErr> for EventErr {
    fn from(e: ReadErr) -> Self {
        Self::ReadErr(e)
    }
}

impl From<MalformedCursorErr> for EventErr {
    fn from(e: MalformedCursorErr) -> Self {
        Self::MalformedCursorErr(e)
    }
}

impl From<CursorExpiredErr> for EventErr {
    fn from(e: CursorExpiredErr) -> Self {
        Self::CursorExpiredErr(e)
    }
}

crate::impl_error!(EventErr {
    InitErr,
    AppendErr,
    ReadErr,
    MalformedCursorErr,
    CursorExpiredErr,
});
