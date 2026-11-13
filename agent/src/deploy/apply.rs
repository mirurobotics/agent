// internal crates
use crate::data_uploads::scan::validate_rules;
use crate::deploy::{errors::*, filesys as dpl_filesys, fsm};
use crate::disk;
use crate::filesys;
use crate::models;
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
    pub deployments: &'a disk::Deployments,
    pub cfg_insts: disk::CfgInstRef<'a>,
    pub releases: &'a disk::Releases,
    pub file_rules: &'a disk::FileRules,
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
    let categorized = read_deployments(args.storage.deployments).await?;

    let mut outcomes = Vec::new();
    let mut tgt_dpld_files: Vec<filesys::File> = Vec::new();
    let hold_removals = match &categorized.target_deployed {
        Some(target_deployed) => {
            tgt_dpld_files = read_cfg_insts(
                args.storage.cfg_insts.index,
                &target_deployed.config_instance_ids,
            )
            .await?
            .iter()
            .map(|ci| filesys::File::new(&ci.filepath))
            .collect();

            let outcome = apply_one(args, target_deployed.clone(), &[]).await;
            let held = outcome.deployment.activity_status != models::DplActivity::Deployed;
            outcomes.push(outcome);
            held
        }
        None => categorized.target_failed,
    };

    let rest = hold_or_mark_removals(args.storage.deployments, categorized, hold_removals).await;
    outcomes.extend(apply_all(args, rest, &tgt_dpld_files).await?);
    Ok(outcomes)
}

/// Holds pending removals, or marks them `Removing` in storage, and returns the
/// deployments to apply after the target. Removals are held while
/// `hold_removals`, so the old deployment's files stay until its replacement is
/// deployed; a replacement that is retrying, cooling down or failed would
/// otherwise leave the device with a partial config. If marking fails, removals
/// are held for this sync and the next sync retries them.
async fn hold_or_mark_removals(
    storage: &disk::Deployments,
    mut categorized: Categorized,
    hold_removals: bool,
) -> Vec<models::Deployment> {
    if hold_removals {
        if !categorized.remove.is_empty() {
            let ids: Vec<&str> = categorized.remove.iter().map(|d| d.id.as_str()).collect();
            info!("holding removal of {ids:?} until the target deployment is deployed");
        }
        return categorized.without_target_deployed_or_removing();
    }
    let remove = std::mem::take(&mut categorized.remove);
    match mark_removing(storage, remove).await {
        Ok(marked) => {
            categorized.remove = marked;
            categorized.without_target_deployed()
        }
        Err(e) => {
            // keep the target's outcome; the next sync retries the removals
            error!("failed to mark deployments as removing: {e}");
            categorized.without_target_deployed_or_removing()
        }
    }
}

async fn read_deployments(storage: &disk::Deployments) -> Result<Categorized, DeployErr> {
    let (target_deployed, target_failed) = find_target_deployed(storage).await?;
    let tgt_dpl_id = target_deployed.as_ref().map(|d| d.id.clone());

    let mut categorized = Categorized {
        none: Vec::new(),
        wait: Vec::new(),
        target_deployed,
        target_failed,
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

/// Returns the deployment to deploy, and whether a deployment meant to be
/// deployed has exhausted its retries.
async fn find_target_deployed(
    storage: &disk::Deployments,
) -> Result<(Option<models::Deployment>, bool), DeployErr> {
    let (failed, target_deployed): (Vec<_>, Vec<_>) = storage
        .find_where(|d| d.target_status == models::DplTarget::Deployed)
        .await?
        .into_iter()
        .partition(|d| d.error_status == models::DplErrStatus::Failed);
    if target_deployed.len() > 1 {
        return Err(ConflictingDeploymentsErr {
            ids: target_deployed.iter().map(|d| d.id.clone()).collect(),
            trace: trace!(),
        }
        .into());
    }
    Ok((target_deployed.into_iter().next(), !failed.is_empty()))
}

async fn read_cfg_insts(
    storage: &disk::CfgInsts,
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
    /// a deployment meant to be deployed has exhausted its retries
    target_failed: bool,
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

    fn without_target_deployed_or_removing(&self) -> Vec<models::Deployment> {
        self.none
            .iter()
            .chain(self.wait.iter())
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

    let result = match validate_file_rules(storage, &deployment).await {
        Ok(()) => dpl_filesys::deploy(&storage.cfg_insts, &deployment).await,
        Err(e) => Err(e),
    };
    match result {
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

/// Rejects a deployment whose release carries an invalid file rule set, before
/// any config instance is written. The sync stores a deployment's release and
/// rules before applying it, so a missing release is an error.
async fn validate_file_rules(
    storage: &Storage<'_>,
    deployment: &models::Deployment,
) -> Result<(), DeployErr> {
    let rules =
        disk::file_rules_for_deployment(storage.releases, storage.file_rules, deployment).await?;
    validate_rules(&rules).map_err(|e| InvalidFileRulesErr {
        deployment_id: deployment.id.clone(),
        reason: e.to_string(),
        trace: trace!(),
    })?;
    Ok(())
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
    storage: &disk::Deployments,
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
    storage: &disk::Deployments,
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

async fn archive(deployments: &disk::Deployments, deployment: models::Deployment) -> Outcome {
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
    storage: &disk::Deployments,
    deployment: &models::Deployment,
) -> Result<(), DeployErr> {
    storage
        .write(
            deployment.id.clone(),
            deployment.clone(),
            disk::deployments::is_dirty,
            filesys::Overwrite::Allow,
        )
        .await
        .map_err(DeployErr::from)
}
