// internal crates
use crate::authn::{self, TokenManagerExt};
use crate::http;
use crate::models;
use crate::services::{deployment as dpl_svc, errors::ServiceErr};
use crate::storage;
use crate::sync;
use backend_api::models as backend_client;

#[allow(async_fn_in_trait)]
pub trait ReleaseFetcher {
    async fn fetch_release(&self, id: &str) -> Result<backend_client::Release, ServiceErr>;
}

pub struct HttpReleaseFetcher<'a> {
    pub client: &'a http::Client,
    pub token_mngr: &'a authn::TokenManager,
}

impl<'a> ReleaseFetcher for HttpReleaseFetcher<'a> {
    async fn fetch_release(&self, id: &str) -> Result<backend_client::Release, ServiceErr> {
        let token = self
            .token_mngr
            .get_token()
            .await
            .map_err(|e| ServiceErr::SyncErr(sync::SyncErr::from(e)))?;
        http::with_retry(|| async { http::releases::get(self.client, id, &[], &token.token).await })
            .await
            .map_err(ServiceErr::from)
    }
}

pub async fn get(releases: &storage::Releases, id: String) -> Result<models::Release, ServiceErr> {
    let release = releases.read(id).await?;
    Ok(release)
}

pub async fn get_current(
    deployments: &storage::Deployments,
    releases: &storage::Releases,
) -> Result<models::Release, ServiceErr> {
    let dpl = dpl_svc::get_current(deployments).await?;
    let release = releases.read(dpl.release_id).await?;
    Ok(release)
}
