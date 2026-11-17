//! Calibration intake and the latest calibrations: stamp a report and queue it
//! for upload, read back the newest calibration for each key, and mark one
//! invalid. Reports aren't checked against Miru's rules here; the uploader
//! drops one Miru rejects.

mod content;
mod create;
mod invalidate;
mod latest;
pub use content::*;
pub use create::*;
pub use invalidate::*;
pub use latest::*;

// internal crates
use crate::calibrations::{CalibrationNotFoundErr, CalibrationsErr};
use crate::trace;

// external crates
use uuid::Uuid;

/// `report_id` as a UUID. An id that is not a UUID is not found, like any
/// other id that is not a latest entry.
fn parse_report_id(report_id: &str) -> Result<Uuid, CalibrationsErr> {
    Uuid::parse_str(report_id).map_err(|_| {
        let err = CalibrationNotFoundErr {
            report_id: report_id.to_string(),
            trace: trace!(),
        };
        err.into()
    })
}
