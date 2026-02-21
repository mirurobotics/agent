// internal crates
use crate::crud::prelude::{Find, Read};
use crate::deploy::errors::{ConflictingDeploymentsErr, DeployErr, EmptyConfigInstancesErr};
use crate::deploy::{
    filesys, fsm,
    observer::{on_update, Observer},
};
use crate::filesys::dir::Dir;
use crate::models::config_instance::{CfgInstID, ConfigInstance};
use crate::models::deployment::{Deployment, DeploymentID, DplTarget};

// external crates
use tracing::{error, info, warn};

pub struct Args<'a, DR, CIR, CR> {
    pub deployments: &'a DR,
    pub cfg_insts: &'a CIR,
    pub contents: &'a CR,
    pub staging_dir: &'a Dir,
    pub target_dir: &'a Dir,
    pub retry_policy: &'a fsm::RetryPolicy,
}

pub async fn apply<DR, CIR, CR>(
    args: &Args<'_, DR, CIR, CR>,
    observers: &mut [&mut dyn Observer],
) -> Result<Vec<Outcome>, DeployErr>
where
    DR: Find<DeploymentID, Deployment>,
    CIR: Read<CfgInstID, ConfigInstance>,
    CR: Read<CfgInstID, String>,
{
    let target_deployed = find_target_deployed(args.deployments).await?;

    match target_deployed {
        // if there is a deployment which wishes to be deployed, we apply it first
        // because the filesystem can only have one deployment deployed at a time.
        // Afterward, we apply all other actionables which will be removed or waiting
        // for cooldown to end
        Some(deployment) => {
            let deployed_id = deployment.id.clone();
            let mut outcomes = vec![apply_one(deployment, args, observers).await];
            outcomes.extend(apply_actionables(args, observers, Some(&deployed_id)).await?);
            Ok(outcomes)
        }
        // if there is no deployment which wishes to be deployed, we need to delete the
        // target directory so that stale deployments are removed
        None => {
            let outcomes = apply_actionables(args, observers, None).await?;
            if let Err(e) = args.target_dir.delete().await {
                debug_assert!(false, "failed to delete target directory: {e}");
                warn!("failed to delete target directory: {e}");
            }
            Ok(outcomes)
        }
    }
}

async fn find_target_deployed<DR>(deployments: &DR) -> Result<Option<Deployment>, DeployErr>
where
    DR: Find<DeploymentID, Deployment>,
{
    let target_deployed = deployments
        .find_where(|d| d.target_status == DplTarget::Deployed)
        .await?;
    if target_deployed.len() > 1 {
        return Err(ConflictingDeploymentsErr {
            ids: target_deployed.iter().map(|d| d.id.clone()).collect(),
        }
        .into());
    }
    Ok(target_deployed.into_iter().next())
}

async fn apply_actionables<DR, CIR, CR>(
    args: &Args<'_, DR, CIR, CR>,
    observers: &mut [&mut dyn Observer],
    exclude_id: Option<&DeploymentID>,
) -> Result<Vec<Outcome>, DeployErr>
where
    DR: Find<DeploymentID, Deployment>,
    CIR: Read<CfgInstID, ConfigInstance>,
    CR: Read<CfgInstID, String>,
{
    let mut outcomes = Vec::new();
    let actionable = args
        .deployments
        .find_where(|d| fsm::next_action(d) != fsm::NextAction::None)
        .await?;
    for deployment in actionable {
        if exclude_id == Some(&deployment.id) {
            continue;
        }
        let outcome = apply_one(deployment, args, observers).await;
        outcomes.push(outcome);
    }
    Ok(outcomes)
}

pub struct Outcome {
    pub deployment: Deployment,
    pub wait: Option<chrono::TimeDelta>,
    pub error: Option<DeployErr>,
}

async fn apply_one<DR, CIR, CR>(
    deployment: Deployment,
    args: &Args<'_, DR, CIR, CR>,
    observers: &mut [&mut dyn Observer],
) -> Outcome
where
    CIR: Read<CfgInstID, ConfigInstance>,
    CR: Read<CfgInstID, String>,
{
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
            deploy(
                args.cfg_insts,
                args.contents,
                args.staging_dir,
                args.target_dir,
                deployment,
                args.retry_policy,
                observers,
            )
            .await
        }
        fsm::NextAction::Remove | fsm::NextAction::Archive => {
            info!("removing '{}'", deployment.id);
            remove(deployment, observers).await
        }
    }
}

// ================================= DEPLOY ======================================== //

async fn deploy<CIR, CR>(
    cfg_insts: &CIR,
    contents: &CR,
    staging_dir: &Dir,
    target_dir: &Dir,
    deployment: Deployment,
    retry_policy: &fsm::RetryPolicy,
    observers: &mut [&mut dyn Observer],
) -> Outcome
where
    CIR: Read<CfgInstID, ConfigInstance>,
    CR: Read<CfgInstID, String>,
{
    debug_assert_eq!(fsm::next_action(&deployment), fsm::NextAction::Deploy);

    if deployment.config_instance_ids.is_empty() {
        let err = EmptyConfigInstancesErr {
            deployment_id: deployment.id.clone(),
        };
        return Outcome {
            deployment,
            wait: None,
            error: Some(err.into()),
        };
    }

    match filesys::deploy(cfg_insts, contents, staging_dir, target_dir, &deployment).await {
        Ok(()) => {
            let deployment = fsm::deploy(deployment);
            match on_update(observers, &deployment).await {
                Ok(_) => Outcome {
                    deployment,
                    wait: None,
                    error: None,
                },
                Err(e) => Outcome {
                    deployment,
                    wait: None,
                    error: Some(e),
                },
            }
        }
        Err(e) => {
            let deployment = fsm::error(deployment, retry_policy, &e, true);
            if let Err(obs_e) = on_update(observers, &deployment).await {
                error!(
                    "failed to update deployment {} after error: {obs_e}",
                    deployment.id
                );
            }
            Outcome {
                deployment,
                wait: None,
                error: Some(e),
            }
        }
    }
}

// ================================= REMOVE ======================================== //

async fn remove(deployment: Deployment, observers: &mut [&mut dyn Observer]) -> Outcome {
    let deployment = fsm::remove(deployment);
    let error = on_update(observers, &deployment).await.err();
    Outcome {
        deployment,
        wait: None,
        error,
    }
}
