// internal crates
use crate::calibrations::{errors::CalibrationsErr, report::ReportMetadata};
use crate::disk::layout::CalibrationsLayout;
use crate::queue;

/// The calibration queue: accepted reports kept on disk until each reaches
/// Miru. See [`queue::DirQueue`] for the durability rules.
pub type DirQueue = queue::DirQueue<ReportMetadata, serde_json::Value>;

/// A calibration as intake hands it to the queue, and the on-disk shape of
/// `content/<entry id>.json`.
pub type Report = queue::Item<ReportMetadata, serde_json::Value>;

/// At most 1,000 reports of up to 2 MiB each (2 GiB at worst). Miru accepts
/// up to 1 MiB of calibration data, and 2 MiB leaves room for its JSON
/// escaping and the report metadata in the same file.
pub const DEFAULT_OPTIONS: queue::DirQueueOptions = queue::DirQueueOptions {
    capacity: 1000,
    max_item_bytes: 2 * 1024 * 1024,
};

/// Open the calibration queue at `layout`, creating it when absent.
pub async fn open(
    layout: &CalibrationsLayout,
    options: queue::DirQueueOptions,
) -> Result<DirQueue, CalibrationsErr> {
    Ok(DirQueue::open(options, layout.queue(), layout.content()).await?)
}
