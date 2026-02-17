// internal crates
use crate::cache::Overwrite;
use crate::crud::prelude::{Find, Read};
use crate::deploy::errors::*;
use crate::deploy::{
    filesys, fsm,
    observer::{on_update, Observer},
};
use crate::models::config_instance::ConfigInstance;
use crate::models::deployment::{Deployment, DeploymentActivityStatus};
use crate::storage::config_instances::{ConfigInstanceCache, ConfigInstanceContentCache};
use crate::storage::deployments::{DeploymentCache, DeploymentCacheEntry};

#[derive(Debug)]
pub struct DeployResults {
    pub to_remove: Vec<Deployment>,
    pub to_deploy: Vec<Deployment>,
}

impl DeployResults {
    pub fn empty() -> Self {
        Self {
            to_remove: Vec::new(),
            to_deploy: Vec::new(),
        }
    }
}

use crate::trace;

// external crates
use async_trait::async_trait;
use tracing::{debug, error, info};

pub fn is_dirty(old: Option<&DeploymentCacheEntry>, new: &Deployment) -> bool {
    let old = match old {
        Some(old) => old,
        None => return true,
    };
    old.is_dirty
        || old.value.activity_status != new.activity_status
        || old.value.error_status != new.error_status
}

pub struct StorageObserver<'a> {
    pub deployment_cache: &'a DeploymentCache,
}

#[async_trait]
impl<'a> Observer for StorageObserver<'a> {
    async fn on_update(&mut self, deployment: &Deployment) -> Result<(), DeployErr> {
        self.deployment_cache
            .write(
                deployment.id.clone(),
                deployment.clone(),
                is_dirty,
                Overwrite::Allow,
            )
            .await
            .map_err(DeployErr::from)
    }
}

pub async fn apply(
    deployment: &Deployment,
    deployment_cache: &DeploymentCache,
    cfg_inst_cache: &ConfigInstanceCache,
    ctx: &filesys::DeployContext<'_, ConfigInstanceContentCache>,
) -> Result<Deployment, DeployErr> {
    debug!("Applying deployment {:?}", deployment.id);

    // observers
    let mut observers: Vec<&mut dyn Observer> = Vec::new();
    let mut storage_observer = StorageObserver { deployment_cache };
    observers.push(&mut storage_observer);

    // Query config instances from cache using IDs
    let mut cfg_insts = Vec::new();
    for cfg_inst_id in &deployment.config_instance_ids {
        match cfg_inst_cache.read_optional(cfg_inst_id.clone()).await {
            Ok(Some(cfg_inst)) => cfg_insts.push(cfg_inst),
            Ok(None) => {
                error!("Config instance {} not found in cache", cfg_inst_id);
                return Err(DeployErr::from(crate::crud::errors::CrudErr::from(
                    crate::cache::errors::CacheErr::CacheElementNotFound(
                        crate::cache::errors::CacheElementNotFound {
                            msg: format!("Config instance {} not found", cfg_inst_id),
                            trace: trace!(),
                        },
                    ),
                )));
            }
            Err(e) => {
                error!(
                    "Failed to read config instance {} from cache: {:?}",
                    cfg_inst_id, e
                );
                return Err(DeployErr::from(e));
            }
        }
    }

    // Filter to only those that need action
    // Note: We're filtering config instances, but the deployment FSM determines the action
    // For now, we'll apply all config instances in the deployment
    let cfg_insts_to_apply = cfg_insts;

    if cfg_insts_to_apply.is_empty() {
        // No config instances need action, just return the deployment
        return Ok(deployment.clone());
    }

    let conflicts = find_all_deployed_deployments(deployment_cache).await?;

    let (deployment_results, result) = apply_deployment(
        deployment.clone(),
        cfg_insts_to_apply,
        conflicts,
        ctx,
        &mut observers,
    )
    .await;

    if let Err(e) = result {
        error!("Error applying deployment {:?}: {:?}", deployment.id, e);
    }

    // Return the updated deployment from results
    // The deployment may be in to_deploy (Deploy) or to_remove (Remove/Archive)
    let updated_deployment = deployment_results
        .to_deploy
        .into_iter()
        .chain(deployment_results.to_remove.into_iter())
        .find(|d| d.id == deployment.id)
        .unwrap_or_else(|| deployment.clone());

    Ok(updated_deployment)
}

async fn apply_deployment(
    deployment: Deployment,
    cfg_insts_to_apply: Vec<ConfigInstance>,
    conflicts: Vec<Deployment>,
    ctx: &filesys::DeployContext<'_, ConfigInstanceContentCache>,
    observers: &mut [&mut dyn Observer],
) -> (DeployResults, Result<(), DeployErr>) {
    match fsm::next_action(&deployment, true) {
        fsm::NextAction::None => (DeployResults::empty(), Ok(())),
        fsm::NextAction::Deploy => {
            deploy_deployment(deployment, cfg_insts_to_apply, conflicts, ctx, observers).await
        }
        fsm::NextAction::Remove => remove_deployment(deployment, observers).await,
        fsm::NextAction::Archive => archive(deployment, observers).await,
        fsm::NextAction::Wait(_) => (DeployResults::empty(), Ok(())),
    }
}

// =================================== DEPLOY ====================================== //
async fn deploy_deployment(
    mut deployment: Deployment,
    cfg_insts_to_deploy: Vec<ConfigInstance>,
    conflicts: Vec<Deployment>,
    ctx: &filesys::DeployContext<'_, ConfigInstanceContentCache>,
    observers: &mut [&mut dyn Observer],
) -> (DeployResults, Result<(), DeployErr>) {
    if fsm::next_action(&deployment, true) != fsm::NextAction::Deploy {
        let next_action = fsm::next_action(&deployment, true);
        return (
            DeployResults::empty(),
            Err(DeployErr::DeploymentNotDeployableErr(
                DeploymentNotDeployableErr {
                    deployment,
                    next_action,
                    trace: trace!(),
                },
            )),
        );
    }

    if cfg_insts_to_deploy.is_empty() {
        // No config instances to deploy, just update the deployment status
        let deployment = fsm::deploy(deployment);
        let result = on_update(observers, &deployment).await;
        return (
            DeployResults {
                to_remove: vec![],
                to_deploy: vec![deployment],
            },
            result,
        );
    }

    deployment.config_instance_ids = cfg_insts_to_deploy.iter().map(|ci| ci.id.clone()).collect();

    let replacement_ids = conflicts.iter().map(|c| &c.id).collect::<Vec<_>>();
    info!(
        "deploying deployment {:?} and removing {:?}",
        deployment.id, replacement_ids
    );

    let updated_to_remove = record_removals(conflicts, observers).await;

    let (updated_deployment, result) =
        filesys::deploy(ctx, deployment, cfg_insts_to_deploy, observers).await;

    (
        DeployResults {
            to_remove: updated_to_remove,
            to_deploy: vec![updated_deployment],
        },
        result,
    )
}

async fn record_removals(
    to_remove: Vec<Deployment>,
    observers: &mut [&mut dyn Observer],
) -> Vec<Deployment> {
    let mut out = Vec::with_capacity(to_remove.len());
    for mut deployment in to_remove {
        deployment = fsm::remove(deployment);
        if let Err(e) = on_update(observers, &deployment).await {
            error!(
                "Error updating removed deployment {:?}: {:?}",
                deployment.id, e
            );
        }
        out.push(deployment);
    }
    out
}

// =================================== REMOVE ====================================== //
async fn remove_deployment(
    deployment: Deployment,
    observers: &mut [&mut dyn Observer],
) -> (DeployResults, Result<(), DeployErr>) {
    if fsm::next_action(&deployment, true) != fsm::NextAction::Remove {
        let next_action = fsm::next_action(&deployment, true);
        return (
            DeployResults::empty(),
            Err(DeployErr::DeploymentNotRemoveableErr(
                DeploymentNotRemoveableErr {
                    deployment,
                    next_action,
                    trace: trace!(),
                },
            )),
        );
    }

    info!("removing deployment {:?}", deployment.id);

    let deployment = fsm::remove(deployment);
    let result = on_update(observers, &deployment).await;
    (
        DeployResults {
            to_remove: vec![deployment],
            to_deploy: vec![],
        },
        result,
    )
}

async fn find_all_deployed_deployments(
    deployment_cache: &DeploymentCache,
) -> Result<Vec<Deployment>, DeployErr> {
    // Find all deployments that are currently deployed
    // Since only one deployment can be deployed at a time, all deployed deployments are conflicts
    deployment_cache
        .find_where(|d| d.activity_status == DeploymentActivityStatus::Deployed)
        .await
        .map_err(DeployErr::from)
}

// =================================== ARCHIVE ===================================== //
async fn archive(
    deployment: Deployment,
    observers: &mut [&mut dyn Observer],
) -> (DeployResults, Result<(), DeployErr>) {
    if fsm::next_action(&deployment, true) != fsm::NextAction::Archive {
        let next_action = fsm::next_action(&deployment, true);
        return (
            DeployResults::empty(),
            Err(DeployErr::DeploymentNotArchiveableErr(
                DeploymentNotArchiveableErr {
                    deployment,
                    next_action,
                    trace: trace!(),
                },
            )),
        );
    }

    info!(
        "Archiving deployment '{}' (it is not currently deployed)",
        deployment.id
    );
    let deployment = fsm::remove(deployment);
    return (
        DeployResults {
            to_remove: vec![deployment.clone()],
            to_deploy: vec![],
        },
        on_update(observers, &deployment).await,
    );
}
