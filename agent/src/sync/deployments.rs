use crate::crud::prelude::*;
use crate::deploy::{apply::apply, filesys, fsm};
use crate::http::deployments::DeploymentsExt;
use crate::models::config_instance::ConfigInstance;
use crate::models::deployment::Deployment;
use crate::storage::config_instances::{ConfigInstanceCache, ConfigInstanceContentCache};
use crate::storage::deployments::DeploymentCache;
use crate::sync::errors::*;
use crate::trace;
use openapi_client::models::{
    DeploymentActivityStatus as BackendActivityStatus, DeploymentListExpansion,
    UpdateDeploymentRequest,
};

// external crates
use tracing::{debug, error};

// =================================== SYNC ======================================== //
pub async fn sync<HTTPClientT: DeploymentsExt>(
    deployment_cache: &DeploymentCache,
    cfg_inst_cache: &ConfigInstanceCache,
    cfg_inst_content_cache: &ConfigInstanceContentCache,
    http_client: &HTTPClientT,
    ctx: &filesys::DeployContext<'_, ConfigInstanceContentCache>,
    token: &str,
) -> Result<(), SyncErr> {
    let mut errors = Vec::new();

    // pull deployments from server
    debug!("Pulling deployments from server");
    let result = pull(
        deployment_cache,
        cfg_inst_cache,
        cfg_inst_content_cache,
        http_client,
        token,
    )
    .await;
    match result {
        Ok(_) => (),
        Err(e) => {
            errors.push(e);
        }
    };

    // read the deployments which need to be applied
    debug!("Reading deployments which need to be applied");
    let deployments_to_apply = deployment_cache
        .find_where(|deployment| fsm::is_action_required(fsm::next_action(deployment, true)))
        .await
        .map_err(|e| {
            SyncErr::CrudErr(Box::new(SyncCrudErr {
                source: e,
                trace: trace!(),
            }))
        })?;

    // apply each deployment
    for deployment in deployments_to_apply {
        debug!("Applying deployment {}", deployment.id);
        match apply(&deployment, deployment_cache, cfg_inst_cache, ctx).await {
            Ok(_updated) => {
                debug!("Successfully applied deployment {}", deployment.id);
            }
            Err(e) => {
                error!("Error applying deployment {}: {:?}", deployment.id, e);
                errors.push(SyncErr::DeployErr(Box::new(SyncDeployErr {
                    source: e,
                    trace: trace!(),
                })));
            }
        }
    }

    // push deployment status updates to server
    debug!("Pushing deployment status updates to server");
    let result = push(deployment_cache, http_client, token).await;
    match result {
        Ok(_) => (),
        Err(e) => {
            errors.push(e);
        }
    };

    if errors.is_empty() {
        Ok(())
    } else {
        Err(SyncErr::SyncErrors(Box::new(SyncErrors {
            source: errors,
            trace: trace!(),
        })))
    }
}

// =================================== PULL ======================================== //
async fn pull<HTTPClientT: DeploymentsExt>(
    deployment_cache: &DeploymentCache,
    cfg_inst_cache: &ConfigInstanceCache,
    cfg_inst_content_cache: &ConfigInstanceContentCache,
    http_client: &HTTPClientT,
    token: &str,
) -> Result<(), SyncErr> {
    // Fetch active deployments (queued + deployed) with config_instances expanded
    let active_deployments = fetch_active_deployments(http_client, token).await?;
    debug!("Found {} active deployments", active_deployments.len(),);

    // Process each deployment
    for backend_deployment in active_deployments {
        let deployment_id = backend_deployment.id.clone();

        // Extract config instances from the expansion
        let backend_config_instances = backend_deployment
            .config_instances
            .clone()
            .unwrap_or_default();

        // Store config instances in the cache
        for backend_ci in &backend_config_instances {
            let ci = ConfigInstance::from_backend(backend_ci.clone());
            let ci_id = ci.id.clone();

            // Store config instance content if expanded
            if let Some(ref content) = backend_ci.content {
                let overwrite = true;
                if let Err(e) = cfg_inst_content_cache
                    .write(ci_id.clone(), content.clone(), |_, _| false, overwrite)
                    .await
                {
                    error!(
                        "Failed to write config instance '{}' content to cache: {}",
                        ci_id, e
                    );
                    continue;
                }
            }

            // Store config instance metadata
            let overwrite = true;
            if let Err(e) = cfg_inst_cache
                .write(ci_id.clone(), ci, |_, _| false, overwrite)
                .await
            {
                error!(
                    "Failed to write config instance '{}' to cache: {}",
                    ci_id, e
                );
            }
        }

        // Convert to internal deployment model
        let new_deployment = Deployment::from_backend(backend_deployment);

        // Check if we already have this deployment cached
        let existing = deployment_cache
            .read_optional(deployment_id.clone())
            .await
            .map_err(|e| {
                SyncErr::CrudErr(Box::new(SyncCrudErr {
                    source: e,
                    trace: trace!(),
                }))
            })?;

        // Merge: preserve agent-side fields (attempts, cooldown) from cached version
        let merged = match existing {
            Some(cached) => Deployment {
                // Update from backend
                description: new_deployment.description,
                status: new_deployment.status,
                activity_status: new_deployment.activity_status,
                error_status: new_deployment.error_status,
                target_status: new_deployment.target_status,
                config_instance_ids: new_deployment.config_instance_ids,
                updated_at: new_deployment.updated_at,
                // Preserve agent-side fields
                attempts: cached.attempts,
                cooldown_ends_at: cached.cooldown_ends_at,
                ..cached
            },
            None => new_deployment,
        };

        // Write to cache
        let overwrite = true;
        if let Err(e) = deployment_cache
            .write(deployment_id.clone(), merged, |_, _| false, overwrite)
            .await
        {
            error!(
                "Failed to write deployment '{}' to cache: {}",
                deployment_id, e
            );
        }
    }

    Ok(())
}

async fn fetch_active_deployments<HTTPClientT: DeploymentsExt>(
    http_client: &HTTPClientT,
    token: &str,
) -> Result<Vec<openapi_client::models::Deployment>, SyncErr> {
    let activity_status_filter = &[
        BackendActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_QUEUED,
        BackendActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_DEPLOYED,
    ];
    let expansions = [DeploymentListExpansion::DEPLOYMENT_LIST_EXPAND_CONFIG_INSTANCES];
    http_client
        .list_all_deployments(activity_status_filter, expansions, token)
        .await
        .map_err(|e| {
            SyncErr::HTTPClientErr(Box::new(SyncHTTPClientErr {
                source: e,
                trace: trace!(),
            }))
        })
}

// =================================== PUSH ======================================== //
async fn push<HTTPClientT: DeploymentsExt>(
    deployment_cache: &DeploymentCache,
    http_client: &HTTPClientT,
    token: &str,
) -> Result<(), SyncErr> {
    // get all dirty (unsynced) deployments
    let dirty_deployments = deployment_cache.get_dirty_entries().await.map_err(|e| {
        SyncErr::CacheErr(Box::new(SyncCacheErr {
            source: e,
            trace: trace!(),
        }))
    })?;
    debug!(
        "Found {} dirty deployments to push",
        dirty_deployments.len(),
    );

    let mut errors = Vec::new();

    for dirty_entry in dirty_deployments {
        let deployment = dirty_entry.value;

        let activity_status = Some(
            crate::models::deployment::DeploymentActivityStatus::to_backend(
                &deployment.activity_status,
            ),
        );
        let error_status = Some(
            crate::models::deployment::DeploymentErrorStatus::to_backend(&deployment.error_status),
        );
        let updates = UpdateDeploymentRequest {
            activity_status,
            error_status,
        };

        debug!(
            "Pushing deployment {} to server with updates: {:?}",
            deployment.id, updates
        );

        if let Err(e) = http_client
            .update_deployment(
                &deployment.id,
                &updates,
                &[] as &[DeploymentListExpansion],
                token,
            )
            .await
            .map_err(|e| {
                SyncErr::HTTPClientErr(Box::new(SyncHTTPClientErr {
                    source: e,
                    trace: trace!(),
                }))
            })
        {
            error!(
                "Failed to push deployment {} to backend: {}",
                deployment.id, e
            );
            errors.push(e);
            continue;
        }

        // update the cache to mark as clean
        let deployment_id = deployment.id.clone();
        if let Err(e) = deployment_cache
            .write(deployment.id.clone(), deployment, |_, _| false, true)
            .await
            .map_err(|e| {
                SyncErr::CacheErr(Box::new(SyncCacheErr {
                    source: e,
                    trace: trace!(),
                }))
            })
        {
            error!(
                "Failed to update cache for deployment {} after push: {}",
                deployment_id, e
            );
            errors.push(e);
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(SyncErr::SyncErrors(Box::new(SyncErrors {
            source: errors,
            trace: trace!(),
        })))
    }
}
