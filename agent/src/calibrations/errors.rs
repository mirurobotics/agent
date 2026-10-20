// internal crates
use crate::buffer::{BufferErr, BufferFullErr, ItemTooLargeErr};
use crate::errors::{Code, HTTPCode};

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

#[derive(Debug, thiserror::Error)]
pub enum CalibrationsErr {
    #[error(transparent)]
    BufferFullErr(CalibrationBufferFullErr),
    #[error(transparent)]
    TooLargeErr(CalibrationTooLargeErr),
    #[error(transparent)]
    BufferErr(BufferErr),
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

crate::impl_error!(CalibrationsErr {
    BufferFullErr,
    TooLargeErr,
    BufferErr,
});
