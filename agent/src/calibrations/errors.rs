// internal crates
use crate::authn::AuthnErr;
use crate::errors::{Code, HTTPCode, Trace};
use crate::filesys::FileSysErr;
use crate::http::HTTPErr;
use crate::queue::{EnqueueErr, QueueFullErr};

pub type SendActorMessageErr = crate::cache::errors::SendActorMessageErr;
pub type ReceiveActorMessageErr = crate::cache::errors::ReceiveActorMessageErr;

/// The calibration queue is at its report cap: 507
/// `calibration_buffer_full`. Nothing was written and nothing already
/// queued was dropped; the caller should retry once uploads drain it.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct CalibrationBufferFullErr(pub QueueFullErr);

impl crate::errors::Error for CalibrationBufferFullErr {
    fn code(&self) -> Code {
        Code::CalibrationBufferFull
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INSUFFICIENT_STORAGE
    }

    fn params(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "max_reports": self.0.capacity,
        }))
    }
}

/// The report is over the per-report size cap: 413 `calibration_too_large`.
/// Nothing was written.
#[derive(Debug, thiserror::Error)]
#[error("calibration report is {report_bytes} bytes, over the {max_bytes}-byte cap")]
pub struct CalibrationTooLargeErr {
    pub report_bytes: u64,
    pub max_bytes: u64,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for CalibrationTooLargeErr {
    fn code(&self) -> Code {
        Code::CalibrationTooLarge
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::PAYLOAD_TOO_LARGE
    }

    fn params(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "max_bytes": self.max_bytes,
        }))
    }
}

/// A buffered report that can never be sent, because its file is missing or
/// not UTF-8.
/// Terminal, so the worker drops it instead of retrying.
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
/// the report is retried, never dropped.
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
    QueueFullErr(CalibrationBufferFullErr),
    #[error(transparent)]
    TooLargeErr(CalibrationTooLargeErr),
    #[error(transparent)]
    FileSysErr(FileSysErr),
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

/// A full queue becomes the calibration-specific 507.
impl From<EnqueueErr> for CalibrationsErr {
    fn from(e: EnqueueErr) -> Self {
        match e {
            EnqueueErr::QueueFullErr(full) => Self::QueueFullErr(CalibrationBufferFullErr(full)),
            EnqueueErr::FileSysErr(e) => Self::FileSysErr(e),
        }
    }
}

impl From<FileSysErr> for CalibrationsErr {
    fn from(e: FileSysErr) -> Self {
        Self::FileSysErr(e)
    }
}

impl From<CalibrationTooLargeErr> for CalibrationsErr {
    fn from(e: CalibrationTooLargeErr) -> Self {
        Self::TooLargeErr(e)
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
    QueueFullErr,
    TooLargeErr,
    FileSysErr,
    InvalidReportErr,
    HTTPErr,
    TokenErr,
    SendActorMessageErr,
    ReceiveActorMessageErr,
});
