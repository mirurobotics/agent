//! Calibration intake: stamp a report and buffer it for upload.
//! Reports aren't checked against Miru's rules here; one Miru rejects is
//! parked by the upload worker.

mod create;

pub use self::create::*;
