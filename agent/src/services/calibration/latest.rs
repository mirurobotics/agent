// internal crates
use crate::calibrations::{LatestCalibrations, LatestFilter, Uploader};
use crate::services::errors::ServiceErr;

/// The latest calibrations `filter` matches. Answers from the device's store
/// and never calls Miru.
pub async fn list_latest(
    uploader: &Uploader,
    filter: LatestFilter,
) -> Result<LatestCalibrations, ServiceErr> {
    Ok(uploader.list_latest(filter).await?)
}
