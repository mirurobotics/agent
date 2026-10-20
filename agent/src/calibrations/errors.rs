// internal crates
use crate::buffer::{BufferErr, BufferFullErr};
use crate::errors::{Code, HTTPCode};

/// The calibration buffer is at its report or byte cap: 507
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
            "max_bytes": self.0.max_bytes,
        }))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CalibrationsErr {
    #[error(transparent)]
    BufferFullErr(CalibrationBufferFullErr),
    #[error(transparent)]
    BufferErr(BufferErr),
}

/// A full buffer becomes the calibration-specific 507; every other buffer
/// error passes through unchanged.
impl From<BufferErr> for CalibrationsErr {
    fn from(e: BufferErr) -> Self {
        match e {
            BufferErr::BufferFullErr(full) => Self::BufferFullErr(CalibrationBufferFullErr(full)),
            other => Self::BufferErr(other),
        }
    }
}

crate::impl_error!(CalibrationsErr {
    BufferFullErr,
    BufferErr,
});
