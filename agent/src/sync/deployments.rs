use crate::deploy::apply::{self, apply};
use crate::filesys::Overwrite;
use crate::http;
use crate::http::config_instances;
use crate::http::deployments;
use crate::models::config_instance::ConfigInstance;
use crate::models::deployment::Deployment;
use crate::storage;
use crate::sync::errors::*;
use crate::trace;
use openapi_client::models::{
    DeploymentActivityStatus as BackendActivityStatus, DeploymentListExpansion,
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

pub type Storage<'a> = apply::Storage<'a>;

pub async fn sync<HTTPClientT: http::ClientI>(
    args: &SyncArgs<'_, HTTPClientT>,
) -> Result<(), SyncErr> {
    let mut errors = Vec::new();

    // pull deployments from server
    debug!("Pulling deployments from server");
    let result = pull(args.http_client, args.storage, args.token).await;
    match result {
        Ok(_) => (),
        Err(e) => {
            errors.push(e);
        }
    };

    // apply deployments
    debug!("Applying deployments");
    let apply_args = apply::Args {
        storage: args.storage,
        opts: args.opts,
    };
    let outcomes = match apply(&apply_args).await {
        Ok(v) => v,
        Err(e) => {
            error!("Failed to apply deployments: {e}");
            errors.push(SyncErr::from(e));
            vec![]
        }
    };
    for outcome in outcomes {
        if let Some(e) = outcome.error {
            error!(
                "Error applying deployment {}: {:?}",
                outcome.deployment.id, e
            );
            errors.push(SyncErr::from(e));
        } else {
            debug!("Successfully applied deployment {}", outcome.deployment.id);
        }
    }

    // push deployment status updates to server
    debug!("Pushing deployment status updates to server");
    let result = push(args.http_client, args.storage.deployments, args.token).await;
    match result {
        Ok(_) => (),
        Err(e) => {
            errors.push(e);
        }
    };

    if errors.is_empty() {
        Ok(())
    } else {
        Err(SyncErr::SyncErrors(SyncErrors {
            errors,
            trace: trace!(),
        }))
    }
}

// =================================== PULL ======================================== //
async fn pull<'a, HTTPClientT: http::ClientI>(
    http_client: &HTTPClientT,
    storage: &Storage<'a>,
    token: &str,
) -> Result<(), SyncErr> {
    let active_deployments = fetch_active_deployments(http_client, token).await?;
    debug!("Found {} active deployments", active_deployments.len());

    for backend_deployment in active_deployments {
        let deployment_id = backend_deployment.id.clone();
        let backend_config_instances = backend_deployment
            .config_instances
            .clone()
            .unwrap_or_default();

        // Fetch and store each config instance's content + metadata individually.
        // Failures are non-fatal: a failed config instance is skipped and retried
        // on the next sync cycle. Both content and metadata are skipped together
        // to avoid half-written state.
        for backend_cfg_inst in &backend_config_instances {
            let cfg_inst = ConfigInstance::from_backend(backend_cfg_inst.clone());
            let _ = store_config_instance(http_client, &storage.cfg_insts, cfg_inst, token).await;
        }

        let new_deployment = Deployment::from_backend(backend_deployment);
        let existing = storage
            .deployments
            .read_optional(deployment_id.clone())
            .await?;
        let merged = merge_deployment(new_deployment, existing);

        if let Err(e) = storage
            .deployments
            .write(
                deployment_id.clone(),
                merged,
                |_, _| false,
                Overwrite::Allow,
            )
            .await
        {
            error!(
                "Failed to write deployment '{}' to storage: {}",
                deployment_id, e
            );
        }
    }

    Ok(())
}

async fn store_config_instance<HTTPClientT: http::ClientI>(
    http_client: &HTTPClientT,
    storage: &storage::CfgInstRef<'_>,
    cfg_inst: ConfigInstance,
    token: &str,
) -> Result<(), ()> {
    let cfg_inst_id = cfg_inst.id.clone();

    let content = config_instances::get_content(
        http_client,
        config_instances::GetContentParams {
            id: &cfg_inst_id,
            token,
        },
    )
    .await
    .map_err(|e| {
        error!(
            "Failed to fetch content for config instance '{}': {}",
            cfg_inst_id, e
        );
    })?;

    storage
        .content
        .write(cfg_inst_id.clone(), content, |_, _| false, Overwrite::Allow)
        .await
        .map_err(|e| {
            error!(
                "Failed to write config instance '{}' content to storage: {}",
                cfg_inst_id, e
            );
        })?;

    storage
        .meta
        .write(
            cfg_inst_id.clone(),
            cfg_inst,
            |_, _| false,
            Overwrite::Allow,
        )
        .await
        .map_err(|e| {
            error!(
                "Failed to write config instance '{}' to storage: {}",
                cfg_inst_id, e
            );
        })?;

    Ok(())
}

/// Merges a deployment from the backend with the cached version (if any),
/// preserving agent-side fields (attempts, cooldown_ends_at).
fn merge_deployment(new: Deployment, cached: Option<Deployment>) -> Deployment {
    match cached {
        Some(cached) => Deployment {
            description: new.description,
            activity_status: new.activity_status,
            error_status: new.error_status,
            target_status: new.target_status,
            config_instance_ids: new.config_instance_ids,
            updated_at: new.updated_at,
            attempts: cached.attempts,
            cooldown_ends_at: cached.cooldown_ends_at,
            ..cached
        },
        None => new,
    }
}

async fn fetch_active_deployments<HTTPClientT: http::ClientI>(
    http_client: &HTTPClientT,
    token: &str,
) -> Result<Vec<openapi_client::models::Deployment>, SyncErr> {
    let activity_status_filter = &[
        BackendActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_QUEUED,
        BackendActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_DEPLOYED,
    ];
    let expansions = &[DeploymentListExpansion::DEPLOYMENT_LIST_EXPAND_CONFIG_INSTANCES];
    deployments::list_all(
        http_client,
        deployments::ListAllParams {
            activity_status: activity_status_filter,
            expansions,
            token,
        },
    )
    .await
    .map_err(SyncErr::from)
}

// =================================== PUSH ======================================== //
async fn push<HTTPClientT: http::ClientI>(
    http_client: &HTTPClientT,
    storage: &storage::Deployments,
    token: &str,
) -> Result<(), SyncErr> {
    // get all dirty (unsynced) deployments
    let dirty_dpls = storage.get_dirty_entries().await?;
    debug!("Found {} dirty deployments to push", dirty_dpls.len(),);

    let mut errors = Vec::new();

    for dirty_entry in dirty_dpls {
        let deployment = dirty_entry.value;

        let activity = Some(crate::models::deployment::DplActivity::to_backend(
            &deployment.activity_status,
        ));
        let error_status = Some(crate::models::deployment::DplErrStatus::to_backend(
            &deployment.error_status,
        ));
        let updates = UpdateDeploymentRequest {
            activity_status: activity,
            error_status,
        };

        debug!(
            "Pushing deployment {} to server with updates: {:?}",
            deployment.id, updates
        );

        let params = deployments::UpdateParams {
            id: &deployment.id,
            updates: &updates,
            expansions: &[],
            token,
        };
        if let Err(e) = deployments::update(http_client, params)
            .await
            .map_err(SyncErr::from)
        {
            error!(
                "Failed to push deployment {} to backend: {}",
                deployment.id, e
            );
            errors.push(e);
            continue;
        }

        // update storage to mark as clean
        let deployment_id = deployment.id.clone();
        if let Err(e) = storage
            .write(
                deployment.id.clone(),
                deployment,
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .map_err(SyncErr::from)
        {
            error!(
                "Failed to update storage for deployment {} after push: {}",
                deployment_id, e
            );
            errors.push(e);
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
