// internal crates
use super::errors::*;
use crate::deploy::apply;
use crate::filesys::Overwrite;
use crate::http;
use crate::models;
use crate::storage;
use crate::trace;
use backend_api::models::{
    self as backend_client, DeploymentActivityStatus as BackendActivityStatus,
    UpdateDeploymentRequest,
};

// external crates
use tracing::{debug, error};

// =================================== SYNC ======================================== //
pub struct SyncArgs<'a, HTTPClientT> {
    pub http_client: &'a HTTPClientT,
    pub storage: &'a Storage<'a>,
    pub opts: &'a apply::DeployOpts,
    pub token: &'a str,
}

pub struct Storage<'a> {
    pub deployments: &'a storage::Deployments,
    pub cfg_insts: storage::CfgInstRef<'a>,
    pub releases: &'a storage::Releases,
    pub git_commits: &'a storage::GitCommits,
}

impl<'a> Storage<'a> {
    pub fn apply_storage(&self) -> apply::Storage<'a> {
        apply::Storage {
            deployments: self.deployments,
            cfg_insts: storage::CfgInstRef {
                meta: self.cfg_insts.meta,
                content: self.cfg_insts.content,
            },
        }
    }
}

pub async fn sync<HTTPClientT: http::ClientI>(
    args: &SyncArgs<'_, HTTPClientT>,
) -> Result<Option<chrono::TimeDelta>, SyncErr> {
    let mut errors = Vec::new();

    debug!("pulling deployments from server");
    if let Err(e) = pull_deployments(args.http_client, args.storage, args.token).await {
        error!("Failed to pull deployments: {e}");
        errors.push(e);
    }

    debug!("pulling content for config instances");
    if let Err(e) = pull_content_for_cfg_insts(args.http_client, args.storage, args.token).await {
        error!("Failed to pull content for config instances: {e}");
        errors.push(e);
    }

    let wait = apply_deployments(args.storage, args.opts, &mut errors).await;

    debug!("pushing deployment status updates to server");
    if let Err(e) = push_deployments(args.http_client, args.storage.deployments, args.token).await {
        errors.push(e);
    }

    if errors.is_empty() {
        Ok(if wait.is_zero() { None } else { Some(wait) })
    } else {
        Err(SyncErr::SyncErrors(SyncErrors {
            errors,
            trace: trace!(),
        }))
    }
}

// =================================== PULL ======================================== //
async fn pull_deployments<'a, HTTPClientT: http::ClientI>(
    http_client: &HTTPClientT,
    storage: &Storage<'a>,
    token: &str,
) -> Result<(), SyncErr> {
    let active_deployments = fetch_active_deployments(http_client, token).await?;
    debug!("found {} active deployments", active_deployments.len());

    for backend_dpl in active_deployments {
        let cfg_insts = backend_dpl.config_instances.clone().ok_or_else(|| {
            SyncErr::CfgInstsNotExpanded(CfgInstsNotExpandedErr {
                deployment_id: backend_dpl.id.clone(),
            })
        })?;
        let cfg_inst_ids = cfg_insts.iter().map(|inst| inst.id.clone()).collect();

        store_expanded_release(storage, &backend_dpl).await?;
        store_deployment(storage.deployments, backend_dpl, cfg_inst_ids).await?;

        for backend_cfg_inst in &cfg_insts {
            let cfg_inst: models::ConfigInstance = backend_cfg_inst.clone().into();
            let cfg_inst_id = cfg_inst.id.clone();
            storage
                .cfg_insts
                .meta
                .write_if_absent(cfg_inst_id, cfg_inst, |_, _| false)
                .await?;
        }
    }

    Ok(())
}

async fn fetch_active_deployments<HTTPClientT: http::ClientI>(
    http_client: &HTTPClientT,
    token: &str,
) -> Result<Vec<backend_api::models::Deployment>, SyncErr> {
    let activity_status_filter = &[
        BackendActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_QUEUED,
        BackendActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_DEPLOYED,
    ];
    let expansions: &[&str] = &["config_instances", "release.git_commit"];
    http::with_retry(|| {
        http::deployments::list_all(
            http_client,
            http::deployments::ListAllParams {
                activity_status: activity_status_filter,
                expansions,
                token,
            },
        )
    })
    .await
    .map_err(SyncErr::from)
}

async fn pull_content_for_cfg_insts<'a, HTTPClientT: http::ClientI>(
    http_client: &HTTPClientT,
    storage: &Storage<'a>,
    token: &str,
) -> Result<(), SyncErr> {
    let deployments = storage.deployments.entries().await?;
    let mut seen = std::collections::HashSet::new();
    let mut errors = Vec::new();

    for deployment in deployments {
        for cfg_inst_id in deployment.value.config_instance_ids.clone() {
            if !seen.insert(cfg_inst_id.clone()) {
                continue;
            }
            if let Err(e) =
                pull_cfg_inst_content(http_client, &storage.cfg_insts, cfg_inst_id, token).await
            {
                error!("Failed to pull content for config instance: {e}");
                errors.push(e);
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(SyncErr::SyncErrors(SyncErrors {
            errors,
            trace: trace!(),
        }))
    }
}

async fn pull_cfg_inst_content<HTTPClientT: http::ClientI>(
    http_client: &HTTPClientT,
    storage: &storage::CfgInstRef<'_>,
    cfg_inst_id: String,
    token: &str,
) -> Result<(), SyncErr> {
    if storage
        .content
        .read_optional(cfg_inst_id.clone())
        .await?
        .is_some()
    {
        return Ok(());
    }

    let content = http::with_retry(|| {
        http::config_instances::get_content(
            http_client,
            http::config_instances::GetContentParams {
                id: &cfg_inst_id,
                token,
            },
        )
    })
    .await?;

    storage
        .content
        .write(cfg_inst_id, content, |_, _| false, Overwrite::Allow)
        .await
        .map_err(SyncErr::from)
}

/// Converts a backend deployment into an agent-side model, merges it with any
/// cached version, and writes it to storage.
///
/// The dirty-flag closure keeps the entry dirty if a prior push attempt failed,
/// ensuring the next push phase retries the update even though the pull just
/// overwrote the value.
async fn store_deployment(
    storage: &storage::Deployments,
    backend_dpl: backend_client::Deployment,
    cfg_inst_ids: Vec<String>,
) -> Result<(), SyncErr> {
    let storage_dpl = models::Deployment::from_backend(backend_dpl, cfg_inst_ids);
    let deployment_id = storage_dpl.id.clone();

    let existing = storage.read_optional(deployment_id.clone()).await?;
    let deployment = resolve_dpl(storage_dpl, existing);

    storage
        .write(
            deployment_id,
            deployment,
            |old, _| old.is_some_and(|entry| entry.is_dirty),
            Overwrite::Allow,
        )
        .await
        .map_err(SyncErr::from)
}

/// Extracts and caches the expanded release and git_commit from a backend
/// deployment before the deployment itself is stored (which only keeps the
/// release_id reference).
///
/// Uses `write_if_absent` because releases and git_commits are immutable on the
/// backend — once created, their fields never change. Skipping writes for
/// already-cached entries avoids unnecessary I/O on every sync cycle.
async fn store_expanded_release(
    storage: &Storage<'_>,
    backend_dpl: &backend_client::Deployment,
) -> Result<(), SyncErr> {
    let Some(backend_release) = backend_dpl.release.as_deref() else {
        return Ok(());
    };

    let release: models::Release = backend_release.clone().into();
    let release_id = release.id.clone();
    storage
        .releases
        .write_if_absent(release_id, release, |_, _| false)
        .await?;

    let Some(Some(backend_gc)) = &backend_release.git_commit else {
        return Ok(());
    };

    let gc: models::GitCommit = (*backend_gc.clone()).into();
    let gc_id = gc.id.clone();
    storage
        .git_commits
        .write_if_absent(gc_id, gc, |_, _| false)
        .await?;

    Ok(())
}

// Cached deployment entries are intentionally authoritative for all fields except
// `target_status`, which is always taken from the backend payload.
//
// This preserves locally derived state (activity/error transitions, attempts,
// cooldown metadata, and dirty-retry context) while still reacting to backend
// target changes.
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

// =================================== APPLY ======================================= //
async fn apply_deployments<'a>(
    storage: &'a Storage<'a>,
    opts: &'a apply::DeployOpts,
    errors: &mut Vec<SyncErr>,
) -> chrono::TimeDelta {
    debug!("applying deployments");
    let apply_stor = storage.apply_storage();
    let apply_args = apply::Args {
        storage: &apply_stor,
        opts,
    };
    let outcomes = match apply::apply(&apply_args).await {
        Ok(v) => v,
        Err(e) => {
            error!("Failed to apply deployments: {e}");
            errors.push(SyncErr::from(e));
            return chrono::TimeDelta::zero();
        }
    };
    let mut wait: Option<chrono::TimeDelta> = None;
    for outcome in outcomes {
        if let Some(e) = outcome.error {
            error!(
                "error applying deployment {}: {:?}",
                outcome.deployment.id, e
            );
            errors.push(SyncErr::from(e));
        } else {
            debug!("successfully applied deployment {}", outcome.deployment.id);
        }
        if let Some(w) = outcome.wait {
            if w <= chrono::TimeDelta::zero() {
                continue;
            }
            wait = Some(match wait {
                Some(cur_wait) => cur_wait.min(w),
                None => w,
            });
        }
    }
    wait.unwrap_or(chrono::TimeDelta::zero())
}

// =================================== PUSH ======================================== //
async fn push_deployments<HTTPClientT: http::ClientI>(
    http_client: &HTTPClientT,
    storage: &storage::Deployments,
    token: &str,
) -> Result<(), SyncErr> {
    let dirty_entries = storage.get_dirty_entries().await?;
    debug!("found {} dirty deployments to push", dirty_entries.len(),);

    let mut errors = Vec::new();

    for dirty_entry in dirty_entries {
        let deployment = dirty_entry.value;
        if let Err(e) = push_deployment(http_client, storage, deployment, token).await {
            errors.push(e);
            continue;
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(SyncErr::SyncErrors(SyncErrors {
            errors,
            trace: trace!(),
        }))
    }
}

async fn push_deployment<HTTPClientT: http::ClientI>(
    http_client: &HTTPClientT,
    storage: &storage::Deployments,
    deployment: models::Deployment,
    token: &str,
) -> Result<(), SyncErr> {
    let activity = Some((&deployment.activity_status).into());
    let error_status = Some((&deployment.error_status).into());
    let payload = UpdateDeploymentRequest {
        activity_status: activity,
        error_status,
        deployed_at: deployment.deployed_at.map(|dt| dt.to_rfc3339()),
        archived_at: deployment.archived_at.map(|dt| dt.to_rfc3339()),
    };

    debug!(
        "pushing deployment '{}' updates to server: {:?}",
        deployment.id, payload
    );

    http::with_retry(|| {
        let params = http::deployments::UpdateParams {
            id: &deployment.id,
            updates: &payload,
            token,
        };
        http::deployments::update(http_client, params)
    })
    .await?;

    // mark as clean in storage
    storage
        .write(
            deployment.id.clone(),
            deployment,
            |_, _| false,
            Overwrite::Allow,
        )
        .await
        .map_err(SyncErr::from)
}
