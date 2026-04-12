// internal crates
use crate::deploy::{errors::*, filesys as dpl_filesys, fsm};
use crate::filesys;
use crate::models;
use crate::storage;
use crate::trace;

// external crates
use chrono::Utc;
use tracing::{error, info};

pub struct DeployOpts {
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
    /// Whether the FSM actually transitioned the deployment state (deploy, remove,
    /// archive). False for no-op and wait outcomes.
    pub transitioned: bool,
}

pub async fn apply(args: &Args<'_>) -> Result<Vec<Outcome>, DeployErr> {
    let mut categorized = read_deployments(args.storage.deployments).await?;

    categorized.remove = mark_removing(args.storage.deployments, categorized.remove).await?;

    match &categorized.target_deployed {
        Some(target_deployed) => {
            let tgt_dpld_files: Vec<filesys::File> = read_cfg_insts(
                args.storage.cfg_insts.meta,
                &target_deployed.config_instance_ids,
            )
            .await?
            .iter()
            .map(|ci| filesys::File::new(&ci.filepath))
            .collect();

            let outcome = apply_one(args, target_deployed.clone(), &[]).await;
            if outcome.error.is_some() {
                // The target deployment failed — skip removals (which could
                // delete files the retrying deployment still needs) but still
                // process archives (pure FSM state transitions) and waits
                // (for accurate cooldown reporting).
                let mut outcomes = vec![outcome];
                let safe: Vec<_> = categorized
                    .archive
                    .iter()
                    .chain(categorized.wait.iter())
                    .cloned()
                    .collect();
                outcomes.extend(apply_all(args, safe, &tgt_dpld_files).await?);
                return Ok(outcomes);
            }

            let mut outcomes = vec![outcome];
            outcomes.extend(
                apply_all(args, categorized.without_target_deployed(), &tgt_dpld_files).await?,
            );
            Ok(outcomes)
        }
        None => apply_all(args, categorized.without_target_deployed(), &[]).await,
    }
}

async fn read_deployments(storage: &storage::Deployments) -> Result<Categorized, DeployErr> {
    let target_deployed = find_target_deployed(storage).await?;
    let tgt_dpl_id = target_deployed.as_ref().map(|d| d.id.clone());

    let mut categorized = Categorized {
        none: Vec::new(),
        wait: Vec::new(),
        target_deployed,
        remove: Vec::new(),
        archive: Vec::new(),
    };

    let deployments = storage
        .find_where(move |d| fsm::next_action(d) != fsm::NextAction::None)
        .await?;

    for dpl in deployments.into_iter() {
        if let Some(id) = &tgt_dpl_id {
            if &dpl.id == id {
                continue;
            }
        }
        match fsm::next_action(&dpl) {
            fsm::NextAction::None => {
                categorized.none.push(dpl.clone());
            }
            fsm::NextAction::Wait(_) => {
                categorized.wait.push(dpl.clone());
            }
            fsm::NextAction::Deploy => {
                if let Some(id) = &tgt_dpl_id {
                    return Err(ConflictingDeploymentsErr {
                        ids: vec![dpl.id.clone(), id.clone()],
                        trace: trace!(),
                    }
                    .into());
                } else {
                    return Err(GenericErr {
                        msg: "found unexpected deployment desiring to be deployed which did not match initial target deployment criteria".to_string(),
                        trace: trace!(),
                    }.into());
                }
            }
            fsm::NextAction::Remove => {
                categorized.remove.push(dpl.clone());
            }
            fsm::NextAction::Archive => {
                categorized.archive.push(dpl.clone());
            }
        }
    }
    Ok(categorized)
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
            trace: trace!(),
        }
        .into());
    }
    Ok(target_deployed.into_iter().next())
}

async fn read_cfg_insts(
    storage: &storage::CfgInsts,
    ids: &[String],
) -> Result<Vec<models::ConfigInstance>, DeployErr> {
    let mut cfg_insts = Vec::with_capacity(ids.len());
    for id in ids {
        let cfg_inst = storage.read(id.clone()).await.map_err(DeployErr::from)?;
        cfg_insts.push(cfg_inst);
    }
    Ok(cfg_insts)
}

struct Categorized {
    none: Vec<models::Deployment>,
    wait: Vec<models::Deployment>,
    target_deployed: Option<models::Deployment>,
    remove: Vec<models::Deployment>,
    archive: Vec<models::Deployment>,
}

impl Categorized {
    fn without_target_deployed(&self) -> Vec<models::Deployment> {
        self.none
            .iter()
            .chain(self.wait.iter())
            .chain(self.remove.iter())
            .chain(self.archive.iter())
            .cloned()
            .collect()
    }
}

async fn apply_all(
    args: &Args<'_>,
    deployments: Vec<models::Deployment>,
    dont_remove: &[filesys::File],
) -> Result<Vec<Outcome>, DeployErr> {
    let mut outcomes = Vec::new();
    for deployment in deployments {
        let outcome = apply_one(args, deployment, dont_remove).await;
        outcomes.push(outcome);
    }
    Ok(outcomes)
}

async fn apply_one(
    args: &Args<'_>,
    deployment: models::Deployment,
    dont_remove: &[filesys::File],
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
        fsm::NextAction::Remove => {
            info!("removing '{}'", deployment.id);
            remove(args.storage, args.opts, deployment, dont_remove).await
        }
        fsm::NextAction::Archive => {
            info!("archiving '{}'", deployment.id);
            archive(args.storage.deployments, deployment).await
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

    match dpl_filesys::deploy(&storage.cfg_insts, &deployment).await {
        Ok(()) => {
            let deployment = fsm::deploy(deployment);
            let error = store_dpl(storage.deployments, &deployment).await.err();
            Outcome {
                deployment,
                wait: None,
                error,
                transitioned: true,
            }
        }
        Err(e) => {
            let deployment = fsm::error(deployment, &opts.retry_policy, &e, true);
            if let Err(write_e) = store_dpl(storage.deployments, &deployment).await {
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

// ================================= REMOVING ====================================== //
async fn mark_removing(
    storage: &storage::Deployments,
    deployments: Vec<models::Deployment>,
) -> Result<Vec<models::Deployment>, DeployErr> {
    let mut marked = Vec::new();
    for old in deployments {
        let new = mark_one_removing(storage, old).await?;
        marked.push(new);
    }
    Ok(marked)
}

async fn mark_one_removing(
    storage: &storage::Deployments,
    deployment: models::Deployment,
) -> Result<models::Deployment, DeployErr> {
    debug_assert_eq!(fsm::next_action(&deployment), fsm::NextAction::Remove);

    if deployment.activity_status == models::DplActivity::Removing {
        return Ok(deployment);
    }
    let deployment = fsm::removing(deployment);
    store_dpl(storage, &deployment).await?;
    Ok(deployment)
}

// ================================== REMOVE ======================================= //
async fn remove(
    storage: &Storage<'_>,
    opts: &DeployOpts,
    deployment: models::Deployment,
    ignored: &[filesys::File],
) -> Outcome {
    debug_assert_eq!(fsm::next_action(&deployment), fsm::NextAction::Remove);

    match dpl_filesys::remove(&storage.cfg_insts, &deployment, ignored).await {
        Ok(()) => {
            let deployment = fsm::archive(deployment);
            let error = store_dpl(storage.deployments, &deployment).await.err();
            Outcome {
                deployment,
                wait: None,
                error,
                transitioned: true,
            }
        }
        Err(e) => {
            let deployment = fsm::error(deployment, &opts.retry_policy, &e, true);
            if let Err(write_e) = store_dpl(storage.deployments, &deployment).await {
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

// ================================= ARCHIVE ======================================= //

async fn archive(deployments: &storage::Deployments, deployment: models::Deployment) -> Outcome {
    debug_assert_eq!(fsm::next_action(&deployment), fsm::NextAction::Archive);

    let deployment = fsm::archive(deployment);
    let error = store_dpl(deployments, &deployment).await.err();
    Outcome {
        deployment,
        wait: None,
        error,
        transitioned: true,
    }
}

// ================================= HELPERS ======================================= //

async fn store_dpl(
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
