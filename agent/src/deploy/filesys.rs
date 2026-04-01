// standard crates
use std::collections::HashSet;

// internal crates
use crate::deploy::errors::{DeployErr, EmptyConfigInstancesErr, InvalidDeploymentTargetErr};
use crate::filesys::{self, WriteOptions};
use crate::models;
use crate::storage;

// external crates
use tracing::{info, warn};

/// Reads the deployment's config instances and writes them directly to their
/// absolute filesystem paths.
pub async fn deploy(
    storage: &storage::CfgInstRef<'_>,
    filesys_root: &filesys::Dir,
    deployment: &models::Deployment,
) -> Result<(), DeployErr> {
    validate_deployment(deployment)?;
    let cfg_insts = read_config_instances(storage.meta, &deployment.config_instance_ids).await?;

    for cfg_inst in &cfg_insts {
        write_file(cfg_inst, storage.content, filesys_root).await?;
    }

    info!(
        "wrote {} config instances to filesystem for deployment '{}'",
        deployment.config_instance_ids.len(),
        deployment.id,
    );

    Ok(())
}

/// Removes config instance files from disk for the given deployment, skipping
/// any filepaths present in the `keep` set.
///
/// This is best-effort: individual file deletion failures are logged as warnings
/// rather than propagated, and metadata read failures cause the entire removal
/// to be skipped (orphan files are preferable to deleting active files).
pub async fn remove(
    storage: &storage::CfgInstRef<'_>,
    filesys_root: &filesys::Dir,
    deployment: &models::Deployment,
    ignore: &HashSet<String>,
) {
    let cfg_insts = match read_config_instances(storage.meta, &deployment.config_instance_ids).await
    {
        Ok(v) => v,
        Err(e) => {
            debug_assert!(false, "failed to read config instances for removal of '{}': {e}", deployment.id);
            warn!(
                "failed to read config instances for removal of '{}': {e}",
                deployment.id
            );
            return;
        }
    };

    for cfg_inst in &cfg_insts {
        if ignore.contains(&cfg_inst.filepath) {
            continue;
        }
        let file = filesys_root.file(&cfg_inst.filepath);
        if let Err(e) = file.delete().await {
            debug_assert!(false, "failed to delete config file '{}': {e}", cfg_inst.filepath);
            warn!(
                "failed to delete config file '{}': {e}",
                cfg_inst.filepath
            );
        }
    }

    info!(
        "removed config instance files for deployment '{}'",
        deployment.id
    );
}

/// Collects the set of filepaths for the given config instance IDs.
pub async fn filepaths(
    storage: &storage::CfgInsts,
    config_instance_ids: &[String],
) -> Result<HashSet<String>, DeployErr> {
    let cfg_insts = read_config_instances(storage, config_instance_ids).await?;
    Ok(cfg_insts.into_iter().map(|ci| ci.filepath).collect())
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

async fn write_file(
    cfg_inst: &models::ConfigInstance,
    content_stor: &storage::CfgInstContent,
    filesys_root: &filesys::Dir,
) -> Result<(), DeployErr> {
    let content = content_stor.read(cfg_inst.id.clone()).await?;
    let dest = filesys_root.file(&cfg_inst.filepath);
    dest.write_string(&content, WriteOptions::OVERWRITE_ATOMIC)
        .await?;
    Ok(())
}
