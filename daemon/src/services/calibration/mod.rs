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
