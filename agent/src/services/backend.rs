// standard crates
use std::sync::Arc;

// internal crates
use crate::authn::{Token, TokenManagerExt};
use crate::http::{self, ClientI};
use crate::services::errors::ServiceErr;
use crate::sync;
use backend_api::models as backend_client;

/// Single seam used by the services layer to fetch resources from the backend
/// on a local-cache miss. Consolidates the previous per-resource fetcher
/// traits (`DeploymentFetcher`, `ReleaseFetcher`, `GitCommitFetcher`) into a
/// single interface so that stubbing one backend stubs them all.
#[allow(async_fn_in_trait)]
pub trait BackendFetcher: Send + Sync {
    async fn fetch_deployment(&self, id: &str) -> Result<backend_client::Deployment, ServiceErr>;
    async fn fetch_release(&self, id: &str) -> Result<backend_client::Release, ServiceErr>;
    async fn fetch_git_commit(&self, id: &str) -> Result<backend_client::GitCommit, ServiceErr>;
}

/// Production `BackendFetcher` wrapping an `http::ClientI` and an
/// `authn::TokenManagerExt`. Holds borrows so handlers can build one per
/// request without moving the client or the token manager.
pub struct HttpBackend<'a, C: ClientI, T: TokenManagerExt> {
    client: &'a C,
    token_mngr: &'a T,
}

impl<'a, C: ClientI, T: TokenManagerExt> HttpBackend<'a, C, T> {
    pub fn new(client: &'a C, token_mngr: &'a T) -> Self {
        Self { client, token_mngr }
    }

    async fn token(&self) -> Result<Arc<Token>, ServiceErr> {
        // TokenManagerExt::get_token returns AuthnErr; route through SyncErr
        // so we don't add a new ServiceErr variant.
        self.token_mngr
            .get_token()
            .await
            .map_err(|e| ServiceErr::SyncErr(sync::SyncErr::from(e)))
    }
}

impl<'a, C: ClientI, T: TokenManagerExt> BackendFetcher for HttpBackend<'a, C, T> {
    async fn fetch_deployment(&self, id: &str) -> Result<backend_client::Deployment, ServiceErr> {
        let token = self.token().await?;
        http::with_retry(|| async {
            http::deployments::get(self.client, id, &["config_instances"], &token.token).await
        })
        .await
        .map_err(ServiceErr::from)
    }

    async fn fetch_release(&self, id: &str) -> Result<backend_client::Release, ServiceErr> {
        let token = self.token().await?;
        http::with_retry(|| async { http::releases::get(self.client, id, &[], &token.token).await })
            .await
            .map_err(ServiceErr::from)
    }

    async fn fetch_git_commit(&self, id: &str) -> Result<backend_client::GitCommit, ServiceErr> {
        let token = self.token().await?;
        http::with_retry(|| async {
            http::git_commits::get(self.client, id, &[], &token.token).await
        })
        .await
        .map_err(ServiceErr::from)
    }
}
