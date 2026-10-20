//! Calibration intake: stamp a report and buffer it for the upload worker.
//! Reports aren't checked against Miru's rules here; one Miru rejects is
//! parked by the upload worker.

mod create;

pub use self::create::*;
