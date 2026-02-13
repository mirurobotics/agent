// internal crates
use crate::crud::prelude::Read;
use crate::deploy::errors::{DeployCrudErr, DeployErr, DeployFileSysErr};
use crate::deploy::fsm;
use crate::deploy::observer::{on_update, Observer};
use crate::filesys::dir::Dir;
use crate::models::config_instance::{ConfigInstance, ConfigInstanceID};
use crate::models::deployment::{Deployment, DeploymentTargetStatus};
use crate::trace;

// standard library
use std::path::PathBuf;

// external crates
use tracing::info;

pub struct DeployContext<'a, R> {
    pub content_reader: &'a R,
    pub deployment_dir: &'a Dir,
    pub staging_dir: &'a Dir,
    pub settings: &'a fsm::Settings,
}

/// Writes the given config instances to the deployment directory (atomic replace),
/// then updates the deployment's state and notifies observers.
pub async fn deploy<R>(
    ctx: &DeployContext<'_, R>,
    deployment: Deployment,
    cfg_insts: Vec<ConfigInstance>,
    observers: &mut [&mut dyn Observer],
) -> (Deployment, Result<(), DeployErr>)
where
    R: Read<ConfigInstanceID, serde_json::Value>,
{
    let write_result = write(
        &cfg_insts,
        ctx.content_reader,
        ctx.staging_dir,
        ctx.deployment_dir,
    )
    .await;

    if let Err(e) = &write_result {
        let increment_attempts = deployment.target_status == DeploymentTargetStatus::Deployed;
        let deployment = fsm::error(deployment, ctx.settings, e, increment_attempts);
        if let Err(obs_e) = on_update(observers, &deployment).await {
            return (deployment, Err(obs_e));
        }
        return (deployment, write_result);
    }

    info!(
        "Deployed deployment '{}' with {} config instances to filesystem",
        deployment.id,
        cfg_insts.len()
    );
    let deployment = fsm::deploy(deployment);
    if let Err(e) = on_update(observers, &deployment).await {
        return (deployment, Err(e));
    }
    (deployment, Ok(()))
}

async fn write<R>(
    cfg_insts: &[ConfigInstance],
    content_fetcher: &R,
    staging_dir: &Dir,
    deployment_dir: &Dir,
) -> Result<(), DeployErr>
where
    R: Read<ConfigInstanceID, serde_json::Value>,
{
    let name = format!(
        "deploy_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let temp_dir = staging_dir.subdir(PathBuf::from(name));
    temp_dir.create(true).await.map_err(wrap_file_sys_err)?;

    for cfg_inst in cfg_insts {
        let content = match content_fetcher.read(cfg_inst.id.clone()).await {
            Ok(c) => c,
            Err(e) => {
                let _ = temp_dir.delete().await;
                return Err(wrap_crud_err(e));
            }
        };

        let dest = temp_dir.file(&cfg_inst.filepath);
        if let Err(e) = dest.write_json(&content, true, true).await {
            let _ = temp_dir.delete().await;
            return Err(wrap_file_sys_err(e));
        }
    }

    if let Err(e) = temp_dir.move_to(deployment_dir, true).await {
        let _ = temp_dir.delete().await;
        return Err(wrap_file_sys_err(e));
    }
    Ok(())
}

fn wrap_file_sys_err(e: crate::filesys::errors::FileSysErr) -> DeployErr {
    DeployErr::FileSysErr(Box::new(DeployFileSysErr {
        source: e,
        trace: trace!(),
    }))
}

fn wrap_crud_err(e: crate::crud::errors::CrudErr) -> DeployErr {
    DeployErr::CrudErr(Box::new(DeployCrudErr {
        source: e,
        trace: trace!(),
    }))
}
