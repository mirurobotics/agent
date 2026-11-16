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

/// The latest calibration for each flat component in `components` (any when
/// empty), ordered by flat component. Answers from the device's store and
/// never calls the control plane.
pub async fn list_latest_flat(
    store: &LatestStore,
    components: &[String],
) -> Result<Vec<ReportMetadata>, ServiceErr> {
    let mut latest = store.list(&Filter::default()).await?;
    if !components.is_empty() {
        latest.retain(|entry| components.contains(&entry.flat_component()));
    }
    latest.sort_by_cached_key(ReportMetadata::flat_component);
    Ok(latest)
}
