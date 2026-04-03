// standard crates
use std::collections::HashSet;

// internal crates
use crate::deploy::{
    errors::{ConflictingDeploymentsErr, DeployErr},
    filesys as dpl_filesys, fsm,
};
use crate::filesys;
use crate::models;
use crate::storage;

// external crates
use chrono::Utc;
use tracing::{error, info, warn};

pub struct DeployOpts {
    pub filesys_root: filesys::Dir,
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
    /// Whether the FSM actually transitioned the deployment state (deploy, remove, archive).
    /// False for no-op and wait outcomes.
    pub transitioned: bool,
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
            // Build keep set: filepaths from the new deployment that must not be
            // deleted when cleaning up old deployments.
            let keep = match dpl_filesys::filepaths(
                args.storage.cfg_insts.meta,
                &deployment.config_instance_ids,
            )
            .await
            {
                Ok(set) => set,
                Err(e) => {
                    warn!("failed to build keep set for stale cleanup: {e}");
                    HashSet::new()
                }
            };
            let mut outcomes = vec![apply_one(args, deployment, &keep).await];
            outcomes.extend(apply_actionables(args, Some(deployed_id), &keep).await?);
            Ok(outcomes)
        }
        // if there is no deployment which wishes to be deployed, nothing to keep
        None => {
            let keep = HashSet::new();
            let outcomes = apply_actionables(args, None, &keep).await?;
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
    keep: &HashSet<String>,
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
        let outcome = apply_one(args, deployment, keep).await;
        outcomes.push(outcome);
    }
    Ok(outcomes)
}

async fn apply_one(
    args: &Args<'_>,
    deployment: models::Deployment,
    keep: &HashSet<String>,
) -> Outcome {
    match fsm::next_action(&deployment) {
        fsm::NextAction::None => {
            info!("'{}' has no next action", deployment.id);
            Outcome {
                deployment,
                wait: None,
                error: None,
                transitioned: false,
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
                transitioned: false,
            }
        }
        fsm::NextAction::Deploy => {
            info!("deploying '{}'", deployment.id);
            deploy(args.storage, args.opts, deployment).await
        }
        fsm::NextAction::Remove | fsm::NextAction::Archive => {
            info!("removing '{}'", deployment.id);
            remove(args.storage, args.opts, deployment, keep).await
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

    match dpl_filesys::deploy(&storage.cfg_insts, &opts.filesys_root, &deployment).await {
        Ok(()) => {
            let deployment = fsm::deploy(deployment);
            let error = write_deployment(storage.deployments, &deployment)
                .await
                .err();
            Outcome {
                deployment,
                wait: None,
                error,
                transitioned: true,
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
                transitioned: true,
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

async fn remove(
    storage: &Storage<'_>,
    opts: &DeployOpts,
    deployment: models::Deployment,
    keep: &HashSet<String>,
) -> Outcome {
    // Step 1: Transition to Removing and persist as a breadcrumb. If the agent
    // crashes mid-deletion the deployment will be stuck in Removing rather than
    // Deployed, making it clear what was attempted.
    let deployment = fsm::removing(deployment);
    if let Err(e) = write_deployment(storage.deployments, &deployment).await {
        error!(
            "failed to write removing status for '{}': {e}",
            deployment.id
        );
    }

    // Step 2: Delete config instance files from disk (best-effort)
    dpl_filesys::remove(&storage.cfg_insts, &opts.filesys_root, &deployment, keep).await;

    // Step 3: Transition to Archived
    let deployment = fsm::remove(deployment);
    let error = write_deployment(storage.deployments, &deployment)
        .await
        .err();
    Outcome {
        deployment,
        wait: None,
        error,
        transitioned: true,
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
            filesys::Overwrite::Allow,
        )
        .await
        .map_err(DeployErr::from)
}
