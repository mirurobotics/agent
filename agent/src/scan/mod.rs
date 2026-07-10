pub mod errors;
// The state types are exercised by their own inline tests but are not yet wired
// into a consumer in this crate; the scanner that uses them lands separately.
#[allow(dead_code)]
pub(crate) mod state;
// The collection scanner is exercised by its own inline tests but is not yet
// wired into a consumer in this crate; the parent scanner actor lands separately.
#[allow(dead_code)]
pub(crate) mod collection;
// The parent scanner actor is exercised by its own inline tests but is not
// yet wired into a consumer in this crate; the driving worker lands separately.
#[allow(dead_code)]
pub(crate) mod scanner;
// The upload-rule resolution functions are exercised by their own inline tests
// but are not yet wired into a consumer in this crate; the scanner that calls
// them lands separately.
#[allow(dead_code)]
pub(crate) mod upload_rules;

pub use self::errors::ScanErr;
