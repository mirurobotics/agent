pub mod errors;
// The state types are exercised by their own inline tests but are not yet wired
// into a consumer in this crate; the scanner that uses them lands separately.
#[allow(dead_code)]
pub(crate) mod state;
// The collection scanner is exercised by its own inline tests but is not yet
// wired into a consumer in this crate; the parent scanner actor lands separately.
#[allow(dead_code)]
pub(crate) mod collection;
// Public (mirroring `sync`) so external drivers and integration tests can name
// the actor surface; the scan driver worker consumes `ScannerExt`.
pub mod scanner;

pub use self::errors::ScanErr;
pub use self::scanner::{ScanEvent, Scanner, ScannerArgs, ScannerExt};
