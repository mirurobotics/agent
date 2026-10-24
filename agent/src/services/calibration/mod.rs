//! Calibration intake: stamp a report and queue it for upload. Reports aren't
//! checked against Miru's rules here; the uploader drops one Miru rejects.

mod create;
pub use create::*;
