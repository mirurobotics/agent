// internal crates
use crate::deploy::errors::{DeployErr, EmptyConfigInstancesErr, InvalidDeploymentTargetErr};
use crate::filesys::{self, Overwrite, WriteOptions};
use crate::models;
use crate::storage;

// external crates
use tracing::{info, warn};

/// Reads the deployment's config instances and writes them to the target directory
/// via an atomic staging-directory swap.
pub async fn deploy(
    storage: &storage::CfgInstRef<'_>,
    staging_dir: &filesys::Dir,
    target_dir: &filesys::Dir,
    deployment: &models::Deployment,
) -> Result<(), DeployErr> {
    validate_deployment(deployment)?;
    let cfg_insts = read_config_instances(storage.meta, &deployment.config_instance_ids).await?;

    write_files(&cfg_insts, storage.content, staging_dir, target_dir).await?;

    info!(
        "wrote {} config instances to filesystem for deployment '{}'",
        deployment.config_instance_ids.len(),
        deployment.id,
    );

    Ok(())
}

fn validate_deployment(deployment: &models::Deployment) -> Result<(), DeployErr> {
    if deployment.config_instance_ids.is_empty() {
        return Err(EmptyConfigInstancesErr {
            deployment_id: deployment.id.clone(),
        }
        .into());
    }

    if deployment.target_status != models::DplTarget::Deployed {
        return Err(InvalidDeploymentTargetErr {
            deployment_id: deployment.id.clone(),
            target_status: deployment.target_status,
        }
        .into());
    }

    Ok(())
}

async fn read_config_instances(
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

async fn write_files(
    cfg_insts: &[models::ConfigInstance],
    content_stor: &storage::CfgInstContent,
    staging_dir: &filesys::Dir,
    target_dir: &filesys::Dir,
) -> Result<(), DeployErr> {
    let temp_dir = create_temp_dir(staging_dir).await?;

    let result: Result<(), DeployErr> = async {
        for cfg_inst in cfg_insts {
            write_file(cfg_inst, content_stor, &temp_dir).await?;
        }
        // we assume the the move_to operation is atomic--if it fails the current
        // directory is assumed to be as it was before the move_to operation.
        temp_dir.move_to(target_dir, Overwrite::Allow).await?;
        Ok(())
    }
    .await;

    if let Err(e) = temp_dir.delete().await {
        debug_assert!(false, "failed to clean up temporary directory: {e}");
        warn!("failed to clean up temporary directory: {e}");
    }

    result
}

async fn create_temp_dir(staging_dir: &filesys::Dir) -> Result<filesys::Dir, DeployErr> {
    let temp_dir = staging_dir.subdir(uuid::Uuid::new_v4().to_string());
    temp_dir.create_if_absent().await?;
    Ok(temp_dir)
}

async fn write_file(
    cfg_inst: &models::ConfigInstance,
    content_stor: &storage::CfgInstContent,
    dest_dir: &filesys::Dir,
) -> Result<(), DeployErr> {
    let content = content_stor.read(cfg_inst.id.clone()).await?;
    let dest = dest_dir.file(&cfg_inst.filepath);
    dest.write_string(&content, WriteOptions::OVERWRITE_ATOMIC)
        .await?;
    Ok(())
}
