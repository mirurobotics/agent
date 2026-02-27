// internal crates
use super::errors::{ConflictingDeploymentsErr, DeployErr};
use super::{filesys as dpl_filesys, fsm};
use crate::filesys;
use crate::filesys::Overwrite;
use crate::models;
use crate::storage;

// external crates
use chrono::Utc;
use tracing::{error, info, warn};

pub struct DeployOpts {
    pub staging_dir: filesys::Dir,
    pub target_dir: filesys::Dir,
    pub retry_policy: fsm::RetryPolicy,
}

pub struct Args<'a> {
    pub storage: &'a Storage<'a>,
    pub opts: &'a DeployOpts,
}

pub struct Storage<'a> {
    pub deployments: &'a storage::Deployments,
    pub cfg_insts: storage::CfgInstRef<'a>,
}

pub struct Outcome {
    pub deployment: models::Deployment,
    pub wait: Option<chrono::TimeDelta>,
    pub error: Option<DeployErr>,
}

type DeploymentID = String;

pub async fn apply(args: &Args<'_>) -> Result<Vec<Outcome>, DeployErr> {
    let target_deployed = find_target_deployed(args.storage.deployments).await?;

    match target_deployed {
        // if there is a deployment which wishes to be deployed, we apply it first
        // because the filesystem can only have one deployment deployed at a time.
        // Afterward, we apply all other actionables which will be removed or waiting
        // for cooldown to end
        Some(deployment) => {
            let deployed_id = deployment.id.clone();
            let mut outcomes = vec![apply_one(args, deployment).await];
            outcomes.extend(apply_actionables(args, Some(deployed_id)).await?);
            Ok(outcomes)
        }
        // if there is no deployment which wishes to be deployed, we need to delete the
        // target directory so that stale deployments are removed
        None => {
            let outcomes = apply_actionables(args, None).await?;
            if let Err(e) = args.opts.target_dir.delete().await {
                debug_assert!(false, "failed to delete target directory: {e}");
                warn!("failed to delete target directory: {e}");
            }
            Ok(outcomes)
        }
    }
}

async fn find_target_deployed(
    storage: &storage::Deployments,
) -> Result<Option<models::Deployment>, DeployErr> {
    let target_deployed = storage
        .find_where(|d| {
            d.target_status == models::DplTarget::Deployed
                && d.error_status != models::DplErrStatus::Failed
        })
        .await?;
    if target_deployed.len() > 1 {
        return Err(ConflictingDeploymentsErr {
            ids: target_deployed.iter().map(|d| d.id.clone()).collect(),
        }
        .into());
    }
    Ok(target_deployed.into_iter().next())
}

async fn apply_actionables(
    args: &Args<'_>,
    exclude_id: Option<DeploymentID>,
) -> Result<Vec<Outcome>, DeployErr> {
    let mut outcomes = Vec::new();
    let actionable = args
        .storage
        .deployments
        .find_where(move |d| {
            fsm::next_action(d) != fsm::NextAction::None && Some(&d.id) != exclude_id.as_ref()
        })
        .await?;
    for deployment in actionable {
        let outcome = apply_one(args, deployment).await;
        outcomes.push(outcome);
    }
    Ok(outcomes)
}

async fn apply_one(args: &Args<'_>, deployment: models::Deployment) -> Outcome {
    match fsm::next_action(&deployment) {
        fsm::NextAction::None => {
            info!("'{}' has no next action", deployment.id);
            Outcome {
                deployment,
                wait: None,
                error: None,
            }
        }
        fsm::NextAction::Wait(wait) => {
            info!(
                "'{}' is waiting for cooldown to end ({} seconds)",
                deployment.id,
                wait.num_seconds()
            );
            Outcome {
                deployment,
                wait: Some(wait),
                error: None,
            }
        }
        fsm::NextAction::Deploy => {
            info!("deploying '{}'", deployment.id);
            deploy(args.storage, args.opts, deployment).await
        }
        fsm::NextAction::Remove | fsm::NextAction::Archive => {
            info!("removing '{}'", deployment.id);
            remove(args.storage.deployments, deployment).await
        }
    }
}

// ================================= DEPLOY ======================================== //

async fn deploy(
    storage: &Storage<'_>,
    opts: &DeployOpts,
    deployment: models::Deployment,
) -> Outcome {
    debug_assert_eq!(fsm::next_action(&deployment), fsm::NextAction::Deploy);

    match dpl_filesys::deploy(
        &storage.cfg_insts,
        &opts.staging_dir,
        &opts.target_dir,
        &deployment,
    )
    .await
    {
        Ok(()) => {
            let deployment = fsm::deploy(deployment);
            let error = write_deployment(storage.deployments, &deployment)
                .await
                .err();
            Outcome {
                deployment,
                wait: None,
                error,
            }
        }
        Err(e) => {
            let deployment = fsm::error(deployment, &opts.retry_policy, &e, true);
            if let Err(write_e) = write_deployment(storage.deployments, &deployment).await {
                error!(
                    "failed to update deployment {} after error: {write_e}",
                    deployment.id
                );
            }
            let wait = remaining_cooldown(&deployment);
            Outcome {
                deployment,
                wait,
                error: Some(e),
            }
        }
    }
}

fn remaining_cooldown(deployment: &models::Deployment) -> Option<chrono::TimeDelta> {
    if deployment.is_in_cooldown() {
        let remaining = deployment
            .cooldown_ends_at
            .signed_duration_since(Utc::now())
            .max(chrono::TimeDelta::zero());
        Some(remaining)
    } else {
        None
    }
}

// ================================= REMOVE ======================================== //

async fn remove(deployments: &storage::Deployments, deployment: models::Deployment) -> Outcome {
    let deployment = fsm::remove(deployment);
    let error = write_deployment(deployments, &deployment).await.err();
    Outcome {
        deployment,
        wait: None,
        error,
    }
}

// ================================= HELPERS ======================================= //

async fn write_deployment(
    storage: &storage::Deployments,
    deployment: &models::Deployment,
) -> Result<(), DeployErr> {
    storage
        .write(
            deployment.id.clone(),
            deployment.clone(),
            storage::deployments::is_dirty,
            Overwrite::Allow,
        )
        .await
        .map_err(DeployErr::from)
}
