// internal crates
use crate::authn::{self, TokenManagerExt};
use crate::cache::errors::{CacheElementNotFound, CacheErr};
use crate::http;
use crate::http::errors::RequestFailed;
use crate::models;
use crate::services::errors::ServiceErr;
use crate::storage;
use crate::sync;
use crate::trace;
use backend_api::models as backend_client;

#[allow(async_fn_in_trait)]
pub trait GitCommitFetcher {
    async fn fetch_git_commit(&self, id: &str) -> Result<backend_client::GitCommit, ServiceErr>;
}

pub struct HttpGitCommitFetcher<'a> {
    pub client: &'a http::Client,
    pub token_mngr: &'a authn::TokenManager,
}

impl<'a> GitCommitFetcher for HttpGitCommitFetcher<'a> {
    async fn fetch_git_commit(&self, id: &str) -> Result<backend_client::GitCommit, ServiceErr> {
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

pub async fn get<F: GitCommitFetcher>(
    git_commits: &storage::GitCommits,
    backend: Option<&F>,
    id: String,
) -> Result<models::GitCommit, ServiceErr> {
    let cached = git_commits.read_optional(id.clone()).await?;
    if let Some(gc) = cached {
        return Ok(gc);
    }
    let Some(backend) = backend else {
        return Err(cache_miss_err(&id, "git_commit"));
    };
    let backend_gc = match backend.fetch_git_commit(&id).await {
        Ok(gc) => gc,
        Err(ServiceErr::SyncErr(sync::SyncErr::AuthnErr(e))) => {
            tracing::debug!(
                error = ?e,
                id = %id,
                "token error during cache-miss fallback; returning NotFound"
            );
            return Err(cache_miss_err(&id, "git_commit"));
        }
        Err(ServiceErr::HTTPErr(http::HTTPErr::RequestFailed(RequestFailed {
            status, ..
        }))) if status == reqwest::StatusCode::NOT_FOUND => {
            return Err(cache_miss_err(&id, "git_commit"));
        }
        Err(other) => return Err(other),
    };
    let gc_model = models::GitCommit::from(backend_gc);
    if let Err(e) = git_commits
        .write_if_absent(id.clone(), gc_model.clone(), |_, _| false)
        .await
    {
        tracing::error!(
            error = ?e,
            id = %id,
            "failed to cache fetched git_commit; returning value anyway"
        );
    }
    Ok(gc_model)
}

fn cache_miss_err(id: &str, kind: &str) -> ServiceErr {
    ServiceErr::CacheErr(CacheErr::CacheElementNotFound(CacheElementNotFound {
        msg: format!("{kind} '{id}' not found in cache"),
        trace: trace!(),
    }))
}
