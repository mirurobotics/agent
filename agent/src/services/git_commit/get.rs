// internal crates
use crate::authn::{self, TokenManagerExt};
use crate::http;
use crate::models;
use crate::services::errors::ServiceErr;
use crate::storage;
use crate::sync;
use backend_api::models as backend_client;

#[allow(async_fn_in_trait)]
pub trait GitCommitFetcher {
    async fn fetch_git_commit(
        &self,
        id: &str,
    ) -> Result<backend_client::GitCommit, ServiceErr>;
}

pub struct HttpGitCommitFetcher<'a> {
    pub client: &'a http::Client,
    pub token_mngr: &'a authn::TokenManager,
}

impl<'a> GitCommitFetcher for HttpGitCommitFetcher<'a> {
    async fn fetch_git_commit(
        &self,
        id: &str,
    ) -> Result<backend_client::GitCommit, ServiceErr> {
        let token = self
            .token_mngr
            .get_token()
            .await
            .map_err(|e| ServiceErr::SyncErr(sync::SyncErr::from(e)))?;
        http::with_retry(|| async {
            http::git_commits::get(self.client, id, &[], &token.token).await
        })
        .await
        .map_err(ServiceErr::from)
    }
}

pub async fn get(
    git_commits: &storage::GitCommits,
    id: String,
) -> Result<models::GitCommit, ServiceErr> {
    let gc = git_commits.read(id).await?;
    Ok(gc)
}
