// internal crates
use crate::authn::{self, TokenManagerExt};
use crate::cache::errors::{CacheElementNotFound, CacheErr};
use crate::http;
use crate::http::errors::RequestFailed;
use crate::models;
use crate::services::{deployment as dpl_svc, errors::ServiceErr};
use crate::storage;
use crate::sync;
use crate::trace;
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

pub async fn get<F: ReleaseFetcher>(
    releases: &storage::Releases,
    backend: Option<&F>,
    id: String,
) -> Result<models::Release, ServiceErr> {
    let cached = releases.read_optional(id.clone()).await?;
    if let Some(rls) = cached {
        return Ok(rls);
    }
    let Some(backend) = backend else {
        return Err(cache_miss_err(&id, "release"));
    };
    let backend_rls = match backend.fetch_release(&id).await {
        Ok(r) => r,
        Err(ServiceErr::SyncErr(sync::SyncErr::AuthnErr(e))) => {
            tracing::debug!(
                error = ?e,
                id = %id,
                "token error during cache-miss fallback; returning NotFound"
            );
            return Err(cache_miss_err(&id, "release"));
        }
        Err(ServiceErr::HTTPErr(http::HTTPErr::RequestFailed(RequestFailed {
            status, ..
        }))) if status == reqwest::StatusCode::NOT_FOUND => {
            return Err(cache_miss_err(&id, "release"));
        }
        Err(other) => return Err(other),
    };
    let rls_model = models::Release::from(backend_rls);
    if let Err(e) = releases
        .write_if_absent(id.clone(), rls_model.clone(), |_, _| false)
        .await
    {
        tracing::error!(
            error = ?e,
            id = %id,
            "failed to cache fetched release; returning value anyway"
        );
    }
    Ok(rls_model)
}

fn cache_miss_err(id: &str, kind: &str) -> ServiceErr {
    ServiceErr::CacheErr(CacheErr::CacheElementNotFound(CacheElementNotFound {
        msg: format!("{kind} '{id}' not found in cache"),
        trace: trace!(),
    }))
}

pub async fn get_current(
    deployments: &storage::Deployments,
    releases: &storage::Releases,
) -> Result<models::Release, ServiceErr> {
    let dpl = dpl_svc::get_current(deployments).await?;
    let release = releases.read(dpl.release_id).await?;
    Ok(release)
}
