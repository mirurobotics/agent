// internal crates
use crate::queue::EnqueueErr;
use crate::errors::{Code, HTTPCode, Trace};
use crate::filesys::FileSysErr;

/// The buffer is at its report or byte cap. Nothing was written and nothing
/// already buffered was dropped; the caller should retry once uploads drain it.
#[derive(Debug, thiserror::Error)]
#[error(
    "calibration buffer is full ({reports}/{max_reports} reports, {bytes}/{max_bytes} bytes); rejected report {report_id} of {report_bytes} bytes"
)]
pub struct BufferFullErr {
    pub report_id: String,
    pub report_bytes: u64,
    pub reports: usize,
    pub max_reports: usize,
    pub bytes: u64,
    pub max_bytes: u64,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for BufferFullErr {
    fn code(&self) -> Code {
        Code::CalibrationBufferFull
    }

    fn http_status(&self) -> HTTPCode {
        HTTPCode::INSUFFICIENT_STORAGE
    }

    fn params(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "max_reports": self.max_reports,
            "max_bytes": self.max_bytes,
        }))
    }
}

/// A report id that cannot safely name a file on every platform. Report ids
/// are minted by the agent, so this is an internal error.
#[derive(Debug, thiserror::Error)]
#[error("invalid calibration report id {report_id:?}")]
pub struct InvalidReportIdErr {
    pub report_id: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for InvalidReportIdErr {}

/// A report id already buffered. Report ids are minted by the agent, so this
/// is an internal error.
#[derive(Debug, thiserror::Error)]
#[error("calibration report {report_id} is already buffered")]
pub struct DuplicateReportErr {
    pub report_id: String,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for DuplicateReportErr {}

#[derive(Debug, thiserror::Error)]
#[error("failed to serialize calibration report {report_id}: {source}")]
pub struct SerializeReportErr {
    pub report_id: String,
    #[source]
    pub source: serde_json::Error,
    pub trace: Box<Trace>,
}

impl crate::errors::Error for SerializeReportErr {}

#[derive(Debug, thiserror::Error)]
pub enum CalibrationsErr {
    #[error(transparent)]
    BufferFullErr(BufferFullErr),
    #[error(transparent)]
    InvalidReportIdErr(InvalidReportIdErr),
    #[error(transparent)]
    DuplicateReportErr(DuplicateReportErr),
    #[error(transparent)]
    SerializeReportErr(SerializeReportErr),
    #[error(transparent)]
    EnqueueErr(EnqueueErr),
    #[error(transparent)]
    FileSysErr(FileSysErr),
}

impl From<FileSysErr> for CalibrationsErr {
    fn from(e: FileSysErr) -> Self {
        Self::FileSysErr(e)
    }
}

impl From<EnqueueErr> for CalibrationsErr {
    fn from(e: EnqueueErr) -> Self {
        Self::EnqueueErr(e)
    }
}

crate::impl_error!(CalibrationsErr {
    BufferFullErr,
    InvalidReportIdErr,
    DuplicateReportErr,
    SerializeReportErr,
    EnqueueErr,
    FileSysErr,
});
