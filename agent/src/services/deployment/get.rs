// internal crates
use crate::authn::{self, TokenManagerExt};
use crate::http;
use crate::models;
use crate::services::errors::ServiceErr;
use crate::storage;
use crate::sync;
use backend_api::models as backend_client;

#[allow(async_fn_in_trait)]
pub trait DeploymentFetcher {
    async fn fetch_deployment(
        &self,
        id: &str,
    ) -> Result<backend_client::Deployment, ServiceErr>;
}

pub struct HttpDeploymentFetcher<'a> {
    pub client: &'a http::Client,
    pub token_mngr: &'a authn::TokenManager,
}

impl<'a> DeploymentFetcher for HttpDeploymentFetcher<'a> {
    async fn fetch_deployment(
        &self,
        id: &str,
    ) -> Result<backend_client::Deployment, ServiceErr> {
        // TokenManager::get_token returns AuthnErr; convert via SyncErr because
        // ServiceErr already implements From<SyncErr> and SyncErr already
        // implements From<AuthnErr>. This avoids adding a new error variant.
        let token = self
            .token_mngr
            .get_token()
            .await
            .map_err(|e| ServiceErr::SyncErr(sync::SyncErr::from(e)))?;
        http::with_retry(|| async {
            http::deployments::get(
                self.client,
                id,
                &["config_instances", "release.git_commit"],
                &token.token,
            )
            .await
        })
        .await
        .map_err(ServiceErr::from)
    }
}

pub async fn get(
    deployments: &storage::Deployments,
    id: String,
) -> Result<models::Deployment, ServiceErr> {
    let dpl = deployments.read(id).await?;
    Ok(dpl)
}

pub async fn get_current(
    deployments: &storage::Deployments,
) -> Result<models::Deployment, ServiceErr> {
    let dpl = deployments
        .find_one("deployed", |d| {
            d.activity_status == models::DplActivity::Deployed
        })
        .await?;
    Ok(dpl)
}
