// internal crates
use crate::authn::AuthnErr;
use crate::buffer::{BufferErr, BufferFullErr, ItemTooLargeErr};
use crate::errors::{Code, HTTPCode, Trace};
use crate::http::HTTPErr;

pub type SendActorMessageErr = crate::cache::errors::SendActorMessageErr;
pub type ReceiveActorMessageErr = crate::cache::errors::ReceiveActorMessageErr;

/// The calibration buffer is at its report cap: 507
/// `calibration_buffer_full`. Nothing was written and nothing already
/// buffered was dropped; the caller should retry once uploads drain it.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct CalibrationBufferFullErr(pub BufferFullErr);

impl crate::errors::Error for CalibrationBufferFullErr {
    fn code(&self) -> Code {
        Code::CalibrationBufferFull
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INSUFFICIENT_STORAGE
    }

    fn params(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "max_reports": self.0.max_items,
        }))
    }
}

/// The report is over the per-report size cap: 413 `calibration_too_large`.
/// Nothing was written.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct CalibrationTooLargeErr(pub ItemTooLargeErr);

impl crate::errors::Error for CalibrationTooLargeErr {
    fn code(&self) -> Code {
        Code::CalibrationTooLarge
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::PAYLOAD_TOO_LARGE
    }

    fn params(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "max_bytes": self.0.max_item_bytes,
        }))
    }
}

/// A buffered report that can never be sent: its content is not a valid
/// `{ format, data }` instance content, or its file cannot be parsed. Terminal,
/// so the worker parks it instead of retrying.
#[derive(Debug, thiserror::Error)]
#[error("invalid calibration report: {reason}")]
pub struct InvalidReportErr {
    pub reason: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for InvalidReportErr {
    fn is_terminal(&self) -> bool {
        true
    }
}

/// Getting a device token for an upload failed. Never terminal, whatever the
/// token request's status: a token failure says nothing about the report, so
/// the report is retried, never parked.
#[derive(Debug, thiserror::Error)]
#[error("failed to get a token to upload a calibration: {0}")]
pub struct TokenErr(pub AuthnErr);

impl crate::errors::Error for TokenErr {
    fn is_network_conn_err(&self) -> bool {
        self.0.is_network_conn_err()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CalibrationsErr {
    #[error(transparent)]
    BufferFullErr(CalibrationBufferFullErr),
    #[error(transparent)]
    TooLargeErr(CalibrationTooLargeErr),
    #[error(transparent)]
    BufferErr(BufferErr),
    #[error(transparent)]
    InvalidReportErr(InvalidReportErr),
    #[error(transparent)]
    HTTPErr(HTTPErr),
    #[error(transparent)]
    TokenErr(TokenErr),
    #[error(transparent)]
    SendActorMessageErr(SendActorMessageErr),
    #[error(transparent)]
    ReceiveActorMessageErr(ReceiveActorMessageErr),
}

/// A full buffer becomes the calibration-specific 507 and an oversized report
/// the 413; every other buffer error passes through unchanged.
impl From<BufferErr> for CalibrationsErr {
    fn from(e: BufferErr) -> Self {
        match e {
            BufferErr::BufferFullErr(full) => Self::BufferFullErr(CalibrationBufferFullErr(full)),
            BufferErr::ItemTooLargeErr(large) => Self::TooLargeErr(CalibrationTooLargeErr(large)),
            other => Self::BufferErr(other),
        }
    }
}

impl From<InvalidReportErr> for CalibrationsErr {
    fn from(e: InvalidReportErr) -> Self {
        Self::InvalidReportErr(e)
    }
}

impl From<HTTPErr> for CalibrationsErr {
    fn from(e: HTTPErr) -> Self {
        Self::HTTPErr(e)
    }
}

impl From<AuthnErr> for CalibrationsErr {
    fn from(e: AuthnErr) -> Self {
        Self::TokenErr(TokenErr(e))
    }
}

crate::impl_error!(CalibrationsErr {
    BufferFullErr,
    TooLargeErr,
    BufferErr,
    InvalidReportErr,
    HTTPErr,
    TokenErr,
    SendActorMessageErr,
    ReceiveActorMessageErr,
});
