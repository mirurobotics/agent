// internal crates
use crate::crud::prelude::Read;
use crate::deploy::errors::DeployErr;
use crate::filesys::dir::Dir;
use crate::filesys::{Overwrite, WriteOptions};
use crate::models::config_instance::{CfgInstID, ConfigInstance};
use crate::models::deployment::{Deployment, DplTarget};

// external crates
use tracing::{info, warn};

/// Reads the deployment's config instances and writes them to the target directory
/// via an atomic staging-directory swap.
pub async fn deploy<CIR, CR>(
    cfg_insts: &CIR,
    contents: &CR,
    staging_dir: &Dir,
    target_dir: &Dir,
    deployment: &Deployment,
) -> Result<(), DeployErr>
where
    CIR: Read<CfgInstID, ConfigInstance>,
    CR: Read<CfgInstID, serde_json::Value>,
{
    debug_assert!(
        !deployment.config_instance_ids.is_empty(),
        "deployment '{}' has no config instances",
        deployment.id,
    );
    debug_assert_eq!(
        deployment.target_status,
        DplTarget::Deployed,
        "deployment '{}' is not targeting deployed status",
        deployment.id,
    );
    let cfg_insts = read_config_instances(cfg_insts, &deployment.config_instance_ids).await?;

    write_files(&cfg_insts, contents, staging_dir, target_dir).await?;

    info!(
        "wrote {} config instances to filesystem for deployment '{}'",
        deployment.config_instance_ids.len(),
        deployment.id,
    );

    Ok(())
}

async fn read_config_instances<CIR>(
    reader: &CIR,
    ids: &[CfgInstID],
) -> Result<Vec<ConfigInstance>, DeployErr>
where
    CIR: Read<CfgInstID, ConfigInstance>,
{
    let mut cfg_insts = Vec::with_capacity(ids.len());
    for id in ids {
        let cfg_inst = reader.read(id.clone()).await.map_err(DeployErr::from)?;
        cfg_insts.push(cfg_inst);
    }
    Ok(cfg_insts)
}

async fn write_files<CR>(
    cfg_insts: &[ConfigInstance],
    content_reader: &CR,
    staging_dir: &Dir,
    target_dir: &Dir,
) -> Result<(), DeployErr>
where
    CR: Read<CfgInstID, serde_json::Value>,
{
    let temp_dir = create_temp_dir(staging_dir).await?;

    let result: Result<(), DeployErr> = async {
        for cfg_inst in cfg_insts {
            write_file(cfg_inst, content_reader, &temp_dir).await?;
        }
        // we assume the the move_to operation is atomic--if it fails the current
        // directory is assumed to be as it was before the move_to operation.
        temp_dir.move_to(target_dir, Overwrite::Allow).await?;
        Ok(())
    }
    .await;

    if let Err(e) = temp_dir.delete().await {
        warn!("failed to clean up temporary directory: {e}");
    }

    result
}

async fn create_temp_dir(staging_dir: &Dir) -> Result<Dir, DeployErr> {
    let temp_dir = staging_dir.subdir(uuid::Uuid::new_v4().to_string());
    temp_dir.create_if_absent().await?;
    Ok(temp_dir)
}

async fn write_file<CR>(
    cfg_inst: &ConfigInstance,
    content_reader: &CR,
    dest_dir: &Dir,
) -> Result<(), DeployErr>
where
    CR: Read<CfgInstID, serde_json::Value>,
{
    let content = content_reader.read(cfg_inst.id.clone()).await?;
    let dest = dest_dir.file(&cfg_inst.filepath);
    dest.write_json(&content, WriteOptions::OVERWRITE_ATOMIC)
        .await?;
    Ok(())
}
