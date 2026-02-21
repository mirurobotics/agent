use crate::crud::prelude::*;
use crate::deploy::{
    apply::{self, apply},
    fsm,
    observer::{self, Observer},
};
use crate::filesys::dir::Dir;
use crate::filesys::Overwrite;
use crate::http;
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
#[allow(clippy::too_many_arguments)]
pub async fn sync<HTTPClientT: http::ClientI>(
    deployment_stor: &storage::Deployments,
    cfg_inst_stor: &storage::CfgInsts,
    cfg_inst_content_stor: &storage::CfgInstContent,
    http_client: &HTTPClientT,
    staging_dir: &Dir,
    target_dir: &Dir,
    retry_policy: &fsm::RetryPolicy,
    token: &str,
) -> Result<(), SyncErr> {
    let mut errors = Vec::new();

    // pull deployments from server
    debug!("Pulling deployments from server");
    let result = pull(
        deployment_stor,
        cfg_inst_stor,
        cfg_inst_content_stor,
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

    // apply deployments
    debug!("Applying deployments");
    let args = apply::Args {
        deployments: deployment_stor,
        cfg_insts: cfg_inst_stor,
        contents: cfg_inst_content_stor,
        target_dir,
        staging_dir,
        retry_policy,
    };
    let mut storage_observer = observer::Storage { deployment_stor };
    let mut observers: Vec<&mut dyn Observer> = vec![&mut storage_observer];
    let outcomes = match apply(&args, &mut observers).await {
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
    let result = push(deployment_stor, http_client, token).await;
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
async fn pull<HTTPClientT: http::ClientI>(
    deployment_stor: &storage::Deployments,
    cfg_inst_stor: &storage::CfgInsts,
    cfg_inst_content_stor: &storage::CfgInstContent,
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

        // Store config instances in storage
        for backend_ci in &backend_config_instances {
            let ci = ConfigInstance::from_backend(backend_ci.clone());
            let ci_id = ci.id.clone();

            // Store config instance content if expanded
            if let Some(ref content) = backend_ci.content {
                if let Err(e) = cfg_inst_content_stor
                    .write(
                        ci_id.clone(),
                        content.clone(),
                        |_, _| false,
                        Overwrite::Allow,
                    )
                    .await
                {
                    error!(
                        "Failed to write config instance '{}' content to storage: {}",
                        ci_id, e
                    );
                    continue;
                }
            }

            // Store config instance metadata
            if let Err(e) = cfg_inst_stor
                .write(ci_id.clone(), ci, |_, _| false, Overwrite::Allow)
                .await
            {
                error!(
                    "Failed to write config instance '{}' to storage: {}",
                    ci_id, e
                );
            }
        }

        // Convert to internal deployment model
        let new_deployment = Deployment::from_backend(backend_deployment);

        // Check if we already have this deployment stored
        let existing = deployment_stor.read_optional(deployment_id.clone()).await?;

        // Merge: preserve agent-side fields (attempts, cooldown) from stored version
        let merged = match existing {
            Some(cached) => Deployment {
                // Update from backend
                description: new_deployment.description,
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

        // Write to storage
        if let Err(e) = deployment_stor
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
    deployment_stor: &storage::Deployments,
    http_client: &HTTPClientT,
    token: &str,
) -> Result<(), SyncErr> {
    // get all dirty (unsynced) deployments
    let dirty_deployments = deployment_stor.get_dirty_entries().await?;
    debug!(
        "Found {} dirty deployments to push",
        dirty_deployments.len(),
    );

    let mut errors = Vec::new();

    for dirty_entry in dirty_deployments {
        let deployment = dirty_entry.value;

        let activity_status = Some(crate::models::deployment::DplActivity::to_backend(
            &deployment.activity_status,
        ));
        let error_status = Some(crate::models::deployment::DplErrStatus::to_backend(
            &deployment.error_status,
        ));
        let updates = UpdateDeploymentRequest {
            activity_status,
            error_status,
        };

        debug!(
            "Pushing deployment {} to server with updates: {:?}",
            deployment.id, updates
        );

        if let Err(e) = deployments::update(
            http_client,
            deployments::UpdateParams {
                id: &deployment.id,
                updates: &updates,
                expansions: &[],
                token,
            },
        )
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
        if let Err(e) = deployment_stor
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
