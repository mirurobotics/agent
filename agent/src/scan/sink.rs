// standard crates
use std::future::Future;
use std::pin::Pin;

// internal crates
use crate::models::FileRule;
use crate::scan::scanner::StableFile;

/// An injected async consumer of newly-stable files, called by the scanner
/// actor inside its scan tick.
///
/// Delivery contract: the scanner calls `on_stable_file` exactly once per
/// newly-stable file and **awaits** the returned future before the tick
/// completes — a slow sink applies backpressure to the tick instead of losing
/// files. Losslessness matters because the scanner's ledger dedups
/// observations (`is_latest_ledger_entry` in `scan::rule`): a stable file is
/// reported once per metadata observation and never re-emitted, so a dropped
/// delivery would be permanent.
///
/// Sinks are infallible from the scanner's perspective: they handle and log
/// their own errors internally. `Send + Sync` so handles can be shared as
/// `Arc<dyn StableFileSink>` across the actor boundary.
pub trait StableFileSink: Send + Sync {
    fn on_stable_file<'a>(
        &'a self,
        file: StableFile,
        rule: &'a FileRule,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
}
