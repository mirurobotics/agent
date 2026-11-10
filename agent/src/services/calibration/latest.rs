// internal crates
use crate::calibrations::{Filter, LatestStore, ReportMetadata};
use crate::services::errors::ServiceErr;

/// The latest calibrations a filter matches.
#[derive(Clone, Debug, PartialEq)]
pub struct LatestCalibrations {
    /// Whether the store has been loaded from Miru.
    pub is_confirmed: bool,
    pub calibrations: Vec<ReportMetadata>,
}

/// The latest calibrations `filter` matches. Answers from the device's store
/// and never calls Miru.
pub async fn list_latest(
    store: &LatestStore,
    filter: &Filter,
) -> Result<LatestCalibrations, ServiceErr> {
    let calibrations = store.list(filter).await?;
    Ok(LatestCalibrations {
        // false until the store is loaded from Miru
        is_confirmed: false,
        calibrations,
    })
}
