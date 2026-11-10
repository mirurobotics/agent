// standard crates
use std::future::Future;

// internal crates
use crate::authn::TokenManagerExt;
use crate::calibrations::{Filter, LatestPuller, ReportMetadata};
use crate::http;
use crate::services::errors::ServiceErr;

/// The latest calibrations `filter` matches, from the device's store, after a
/// pull from the control plane if none has succeeded this run. Shutdown stops
/// the pull, not the list.
pub async fn list_latest<HTTPClientT, TokenManagerT>(
    puller: &LatestPuller<HTTPClientT, TokenManagerT>,
    filter: &Filter,
    shutdown: impl Future<Output = ()>,
) -> Result<Vec<ReportMetadata>, ServiceErr>
where
    HTTPClientT: http::ClientI,
    TokenManagerT: TokenManagerExt,
{
    puller.ensure_pulled(shutdown).await;
    Ok(puller.store().list(filter).await?)
}
