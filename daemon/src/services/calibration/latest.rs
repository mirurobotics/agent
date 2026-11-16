// internal crates
use crate::calibrations::{Filter, LatestStore, ReportMetadata};
use crate::services::errors::ServiceErr;

/// The latest calibrations `filter` matches. Answers from the device's store
/// and never calls Miru.
pub async fn list_latest(
    store: &LatestStore,
    filter: &Filter,
) -> Result<Vec<ReportMetadata>, ServiceErr> {
    Ok(store.list(filter).await?)
}
