//! Calibration intake: validate a report against Miru's rules, stamp it, and
//! buffer it for the upload worker.

mod create;
pub mod validate;

pub use self::create::*;
