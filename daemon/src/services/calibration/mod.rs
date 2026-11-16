//! Calibration intake and the latest calibrations: stamp a report and queue it
//! for upload, and read back the newest calibration for each key. Reports
//! aren't checked against Miru's rules here; the uploader drops one Miru
//! rejects.

mod content;
mod create;
mod latest;
pub use content::*;
pub use create::*;
pub use latest::*;
