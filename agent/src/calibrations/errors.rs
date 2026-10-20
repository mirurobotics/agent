// internal crates
use crate::errors::{Code, HTTPCode};
use crate::queue::{DirQueueErr, ItemTooLargeErr, QueueFullErr};

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
    QueueFullErr(CalibrationBufferFullErr),
    #[error(transparent)]
    TooLargeErr(CalibrationTooLargeErr),
    #[error(transparent)]
    QueueErr(DirQueueErr),
}

/// A full queue becomes the calibration-specific 507 and an oversized report
/// the 413; every other queue error passes through unchanged.
impl From<DirQueueErr> for CalibrationsErr {
    fn from(e: DirQueueErr) -> Self {
        match e {
            DirQueueErr::QueueFullErr(full) => Self::QueueFullErr(CalibrationBufferFullErr(full)),
            DirQueueErr::ItemTooLargeErr(large) => Self::TooLargeErr(CalibrationTooLargeErr(large)),
            other => Self::QueueErr(other),
        }
    }
}

crate::impl_error!(CalibrationsErr {
    QueueFullErr,
    TooLargeErr,
    QueueErr,
});
