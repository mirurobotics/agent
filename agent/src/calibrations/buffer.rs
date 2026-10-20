// internal crates
use crate::buffer;
use crate::calibrations::{errors::CalibrationsErr, job::CalibrationJob};
use crate::disk::layout::CalibrationsLayout;

/// The calibration buffer: accepted reports kept on disk until each reaches
/// Miru. See [`buffer::Buffer`] for the durability rules.
pub type Buffer = buffer::Buffer<CalibrationJob, serde_json::Value>;

/// A calibration as intake hands it to the buffer, and the on-disk shape of
/// `content/<entry id>.json`.
pub type Report = buffer::Item<CalibrationJob, serde_json::Value>;

/// At most 1,000 reports of up to 2 MiB each (2 GiB at worst). Miru accepts
/// up to 1 MiB of calibration data, and 2 MiB leaves room for its JSON
/// escaping and the job metadata in the same file.
pub const DEFAULT_OPTIONS: buffer::BufferOptions = buffer::BufferOptions {
    max_items: 1000,
    max_item_bytes: 2 * 1024 * 1024,
};

/// Open the calibration buffer at `layout`, creating it when absent.
pub async fn open(
    layout: &CalibrationsLayout,
    options: buffer::BufferOptions,
) -> Result<Buffer, CalibrationsErr> {
    Ok(Buffer::open(layout.queue(), layout.content(), options).await?)
}
