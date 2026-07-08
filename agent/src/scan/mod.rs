pub mod errors;
// The state types are exercised by their own inline tests but are not yet wired
// into a consumer in this crate; the scanner that uses them lands separately.
#[allow(dead_code)]
pub(crate) mod state;

pub use self::errors::ScanErr;
