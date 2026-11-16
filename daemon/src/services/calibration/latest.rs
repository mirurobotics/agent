// standard crates
use std::collections::BTreeMap;

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
/// empty), as the control plane keys them. Answers from the device's store and
/// never calls the control plane.
pub async fn list_latest_flat(
    store: &LatestStore,
    components: &[String],
) -> Result<Vec<ReportMetadata>, ServiceErr> {
    let mut latest: BTreeMap<String, ReportMetadata> = BTreeMap::new();
    for entry in store.list(&Filter::default()).await? {
        let component = entry.flat_component();
        if !components.is_empty() && !components.contains(&component) {
            continue;
        }
        let kept = latest.get(&component);
        if kept.is_none_or(|kept| entry.reported_at >= kept.reported_at) {
            latest.insert(component, entry);
        }
    }
    Ok(latest.into_values().collect())
}
