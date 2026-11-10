// internal crates
use crate::authn::TokenManagerExt;
use crate::calibrations::{Filter, LatestPuller, LatestStore, ReportMetadata};
use crate::http;
use crate::services::errors::ServiceErr;

/// The latest calibrations `filter` matches, from the device's store, after a
/// pull from the control plane if none has succeeded this run.
pub async fn list_latest<HTTPClientT, TokenManagerT>(
    store: &LatestStore,
    puller: &LatestPuller<HTTPClientT, TokenManagerT>,
    filter: &Filter,
) -> Result<Vec<ReportMetadata>, ServiceErr>
where
    HTTPClientT: http::ClientI,
    TokenManagerT: TokenManagerExt,
{
    puller.ensure_pulled().await;
    Ok(store.list(filter).await?)
}
