pub mod errors;
// State types persisted by the scanner actor (candidate ledgers, deployed set).
pub(crate) mod state;
// Per-collection scanner used by the parent scanner actor.
pub(crate) mod collection;
// Public (mirroring `sync`) so external drivers and integration tests can name
// the actor surface; the scan driver worker consumes `ScannerExt`.
pub mod scanner;

pub use self::errors::ScanErr;
pub use self::scanner::{ScanEvent, Scanner, ScannerArgs, ScannerExt};
