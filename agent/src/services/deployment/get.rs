// internal crates
use crate::authn::{self, TokenManagerExt};
use crate::cache::errors::{CacheElementNotFound, CacheErr};
use crate::filesys::Overwrite;
use crate::http;
use crate::http::errors::RequestFailed;
use crate::models;
use crate::services::errors::ServiceErr;
use crate::storage;
use crate::sync;
use crate::trace;
use backend_api::models as backend_client;

#[allow(async_fn_in_trait)]
pub trait DeploymentFetcher {
    async fn fetch_deployment(&self, id: &str) -> Result<backend_client::Deployment, ServiceErr>;
}

pub struct HttpDeploymentFetcher<'a> {
    pub client: &'a http::Client,
    pub token_mngr: &'a authn::TokenManager,
}

impl<'a> DeploymentFetcher for HttpDeploymentFetcher<'a> {
    async fn fetch_deployment(&self, id: &str) -> Result<backend_client::Deployment, ServiceErr> {
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

pub async fn get<F: DeploymentFetcher>(
    deployments: &storage::Deployments,
    releases: &storage::Releases,
    git_commits: &storage::GitCommits,
    backend: Option<&F>,
    id: String,
) -> Result<models::Deployment, ServiceErr> {
    let cached = deployments.read_optional(id.clone()).await?;
    if let Some(dpl) = cached {
        return Ok(dpl);
    }
    let Some(backend) = backend else {
        return Err(cache_miss_err(&id, "deployment"));
    };
    let backend_dpl = match backend.fetch_deployment(&id).await {
        Ok(d) => d,
        Err(ServiceErr::SyncErr(sync::SyncErr::AuthnErr(e))) => {
            // Token failure: AuthnErr -> SyncErr -> ServiceErr::SyncErr.
            // Falls through to cache-miss error per the Decision Log: clients
            // benefit more from "not found" than from internal auth state.
            tracing::debug!(
                error = ?e,
                id = %id,
                "token error during cache-miss fallback; returning NotFound"
            );
            return Err(cache_miss_err(&id, "deployment"));
        }
        Err(ServiceErr::HTTPErr(http::HTTPErr::RequestFailed(RequestFailed {
            status, ..
        }))) if status == reqwest::StatusCode::NOT_FOUND => {
            return Err(cache_miss_err(&id, "deployment"));
        }
        Err(other) => return Err(other),
    };

    // Re-cache deployment (preserving local state if a sync raced us).
    // backend_dpl.config_instances is Option<Vec<ConfigInstance>>; when we
    // request expand=config_instances the backend must populate it. If it
    // doesn't, that's the same contract violation the syncer reports via
    // SyncErr::CfgInstsNotExpanded — reuse that variant.
    let cfg_insts = backend_dpl.config_instances.as_ref().ok_or_else(|| {
        ServiceErr::SyncErr(sync::SyncErr::CfgInstsNotExpanded(
            sync::errors::CfgInstsNotExpandedErr {
                deployment_id: backend_dpl.id.clone(),
            },
        ))
    })?;
    let cfg_inst_ids: Vec<String> = cfg_insts.iter().map(|ci| ci.id.clone()).collect();

    // Cache the expanded release if present. Duplicated inline from
    // sync::deployments::store_expanded_release per the Decision Log to keep
    // the service layer independent of sync internals. Mirrors the sync
    // ordering: store expanded release BEFORE consuming backend_dpl into the
    // deployment model.
    if let Some(backend_release) = backend_dpl.release.as_deref() {
        let release: models::Release = backend_release.clone().into();
        let release_id = release.id.clone();
        if let Err(e) = releases
            .write_if_absent(release_id, release, |_, _| false)
            .await
        {
            tracing::error!("failed to cache expanded release on cache-miss: {e}");
        }
        // Cache the expanded git_commit if present.
        if let Some(Some(backend_gc)) = &backend_release.git_commit {
            let gc: models::GitCommit = (*backend_gc.clone()).into();
            let gc_id = gc.id.clone();
            if let Err(e) = git_commits.write_if_absent(gc_id, gc, |_, _| false).await {
                tracing::error!("failed to cache expanded git_commit on cache-miss: {e}");
            }
        }
    }

    let new_dpl = models::Deployment::from_backend(backend_dpl, cfg_inst_ids);
    let existing = deployments.read_optional(id.clone()).await.ok().flatten();
    let merged = resolve_dpl(new_dpl, existing);
    if let Err(e) = deployments
        .write(
            id.clone(),
            merged.clone(),
            |old, _| old.is_some_and(|e| e.is_dirty),
            Overwrite::Allow,
        )
        .await
    {
        tracing::error!(
            error = ?e,
            id = %id,
            "failed to cache fetched deployment; returning value anyway"
        );
    }

    Ok(merged)
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

// Inlined from sync::deployments::resolve_dpl per the Decision Log:
// we duplicate this 5-line merge instead of promoting the sync function
// to pub(crate), to keep the service layer free of sync dependencies.
fn resolve_dpl(new: models::Deployment, cached: Option<models::Deployment>) -> models::Deployment {
    match cached {
        Some(cached) => models::Deployment {
            target_status: new.target_status,
            updated_at: new.updated_at,
            ..cached
        },
        None => new,
    }
}

fn cache_miss_err(id: &str, kind: &str) -> ServiceErr {
    ServiceErr::CacheErr(CacheErr::CacheElementNotFound(CacheElementNotFound {
        msg: format!("{kind} '{id}' not found in cache"),
        trace: trace!(),
    }))
}
