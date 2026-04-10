// standard crates
use std::path::Component;

// internal crates
use crate::deploy::errors::*;
use crate::filesys::{self, errors::FileSysErr, PathExt, WriteOptions};
use crate::models;
use crate::storage;
use crate::trace;

// external crates
use tracing::{error, info, warn};

/// Reads the deployment's config instances and writes them to their filesystem
/// destinations using a snapshot+atomic-rename loop with rollback on partial failure.
pub async fn deploy(
    storage: &storage::CfgInstRef<'_>,
    deployment: &models::Deployment,
) -> Result<(), DeployErr> {
    validate_deploy_target(deployment)?;
    validate_has_cfg_insts(deployment)?;

    let cfg_insts = read_cfg_insts(storage.meta, &deployment.config_instance_ids).await?;
    validate_cfg_insts(&cfg_insts)?;

    write_cfg_insts(&cfg_insts, storage.content).await
}

fn validate_has_cfg_insts(deployment: &models::Deployment) -> Result<(), DeployErr> {
    if deployment.config_instance_ids.is_empty() {
        return Err(EmptyConfigInstancesErr {
            deployment_id: deployment.id.clone(),
        }
        .into());
    }
    Ok(())
}

fn validate_deploy_target(deployment: &models::Deployment) -> Result<(), DeployErr> {
    if deployment.target_status != models::DplTarget::Deployed {
        return Err(InvalidDeploymentTargetErr {
            deployment_id: deployment.id.clone(),
            target_status: deployment.target_status,
        }
        .into());
    }
    Ok(())
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

fn validate_cfg_insts(cfg_insts: &[models::ConfigInstance]) -> Result<(), DeployErr> {
    for cfg_inst in cfg_insts {
        validate_filepath(&filesys::File::new(&cfg_inst.filepath))?;
    }
    Ok(())
}

fn validate_filepath(file: &filesys::File) -> Result<(), DeployErr> {
    if !file.is_absolute() {
        return Err(DeployErr::PathNotAllowed(PathNotAllowedErr {
            filepath: file.path().display().to_string(),
            reason: "filepath is not absolute".to_string(),
            trace: trace!(),
        }));
    }

    if file
        .path()
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err(DeployErr::PathNotAllowed(PathNotAllowedErr {
            filepath: file.path().display().to_string(),
            reason: "filepath contains parent traversal".to_string(),
            trace: trace!(),
        }));
    }

    Ok(())
}

async fn write_cfg_insts(
    cfg_insts: &[models::ConfigInstance],
    content_stor: &storage::CfgInstContent,
) -> Result<(), DeployErr> {
    let mut snapshots: Vec<Snapshot> = Vec::with_capacity(cfg_insts.len());
    if let Err(e) = write_cfg_insts_impl(&mut snapshots, cfg_insts, content_stor).await {
        rollback(&snapshots).await;
        return Err(e);
    }
    Ok(())
}

async fn write_cfg_insts_impl(
    snapshots: &mut Vec<Snapshot>,
    cfg_insts: &[models::ConfigInstance],
    content_stor: &storage::CfgInstContent,
) -> Result<(), DeployErr> {
    for cfg_inst in cfg_insts {
        let dest = filesys::File::new(&cfg_inst.filepath);
        let content = content_stor.read(cfg_inst.id.clone()).await?;
        info!(
            "writing config instance {} to {}",
            cfg_inst.id,
            dest.path().display()
        );
        let snapshot = snapshot(&dest).await?;
        snapshots.push(snapshot);

        dest.write_string(&content, WriteOptions::OVERWRITE_ATOMIC)
            .await?;
    }

    cleanup_backups(snapshots).await;
    Ok(())
}

/// Per-destination snapshot captured before a write is attempted. Private
/// to this module so the rollback algorithm is not exposed to callers.
enum Snapshot {
    DidNotExist {
        dst: filesys::File,
    },
    Existed {
        dst: filesys::File,
        backup: filesys::File,
    },
}

async fn snapshot(dst: &filesys::File) -> Result<Snapshot, FileSysErr> {
    if !dst.exists() {
        return Ok(Snapshot::DidNotExist { dst: dst.clone() });
    }

    let backup = backup_location(dst)?;
    dst.copy_to(&backup, filesys::Overwrite::Allow).await?;

    Ok(Snapshot::Existed {
        dst: dst.clone(),
        backup,
    })
}

fn backup_location(dst: &filesys::File) -> Result<filesys::File, FileSysErr> {
    let parent = dst.parent()?;
    let name = dst.name()?;
    let sibling_name = format!("miru.backup.{}", name);
    Ok(parent.file(&sibling_name))
}

async fn rollback(snapshots: &[Snapshot]) {
    for snapshot in snapshots.iter().rev() {
        if let Err(e) = rollback_snapshot(snapshot).await {
            error!("failed to rollback snapshot: {e}");
        }
    }
}

async fn rollback_snapshot(snapshot: &Snapshot) -> Result<(), FileSysErr> {
    match snapshot {
        Snapshot::Existed { dst, backup } => backup.move_to(dst, filesys::Overwrite::Allow).await,
        Snapshot::DidNotExist { dst } => dst.delete().await,
    }
}

/// Best-effort housekeeping that removes the backup file from each
/// `Existed` snapshot on the success path. `NotFound` is ignored; other
/// failures are logged at `warn!` level. Never returns an error.
async fn cleanup_backups(snapshots: &[Snapshot]) {
    for snapshot in snapshots {
        if let Snapshot::Existed { backup, .. } = snapshot {
            if let Err(e) = backup.delete().await {
                warn!(
                    "failed to remove snapshot backup file '{}': {}",
                    backup.path().display(),
                    e,
                );
            }
        }
    }
}

// ================================= REMOVE ======================================== //
pub async fn remove(
    storage: &storage::CfgInstRef<'_>,
    deployment: &models::Deployment,
    keeps: &[filesys::File],
) -> Result<(), DeployErr> {
    if deployment.config_instance_ids.is_empty() {
        return Ok(());
    }
    let cfg_insts = read_cfg_insts(storage.meta, &deployment.config_instance_ids).await?;
    validate_cfg_insts(&cfg_insts)?;

    remove_cfg_insts(&cfg_insts, keeps).await;
    Ok(())
}

async fn remove_cfg_insts(cfg_insts: &[models::ConfigInstance], keeps: &[filesys::File]) {
    for cfg_inst in cfg_insts {
        let dest = filesys::File::new(&cfg_inst.filepath);
        if keeps.contains(&dest) {
            continue;
        }
        if let Err(e) = dest.delete().await {
            error!(
                "failed to remove config instance {} at {}: {e}",
                cfg_inst.id,
                dest.path().display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    // standard crates
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    // internal crates
    use super::*;
    use crate::filesys;

    // ============================= validate_filepath ============================= //

    #[test]
    fn validate_filepath_accepts_clean_absolute_path() {
        let f = filesys::File::new(PathBuf::from("/etc/myapp/config.json"));
        assert!(validate_filepath(&f).is_ok());
    }

    #[test]
    fn validate_filepath_rejects_relative_path() {
        let f = filesys::File::new(PathBuf::from("v1/motion-control.json"));
        match validate_filepath(&f) {
            Err(DeployErr::PathNotAllowed(e)) => {
                assert!(
                    e.reason.contains("not absolute"),
                    "got reason: {}",
                    e.reason
                );
                assert_eq!(e.filepath, "v1/motion-control.json");
            }
            other => panic!("expected PathNotAllowed, got {other:?}"),
        }
    }

    #[test]
    fn validate_filepath_rejects_parent_traversal() {
        let f = filesys::File::new(PathBuf::from("/etc/myapp/../passwd"));
        match validate_filepath(&f) {
            Err(DeployErr::PathNotAllowed(e)) => {
                assert!(
                    e.reason.contains("parent traversal"),
                    "got reason: {}",
                    e.reason
                );
            }
            other => panic!("expected PathNotAllowed, got {other:?}"),
        }
    }

    // ============================= rollback ============================= //

    #[cfg(unix)]
    #[tokio::test]
    async fn rollback_returns_errors_when_restores_fail_synthetic() {
        let tmp = tempfile::tempdir().unwrap();

        // Asymmetric setup: rollback iterates `snapshots.iter().rev()`, so
        // the LAST entry in the vec is processed first. We put the failing
        // Existed snapshot last (so reversed iteration hits it first) and
        // the writable DidNotExist snapshot first (so reversed iteration
        // hits it second). The load-bearing assertion below — that
        // dne_parent/dst.json was deleted — only holds if rollback walked
        // PAST the failing first iteration to reach the second one. If
        // rollback bailed on the first error, dne_parent/dst.json would
        // still be on disk and the assertion would fail.
        let existed_parent = tmp.path().join("existed_parent");
        let dne_parent = tmp.path().join("dne_parent");
        std::fs::create_dir_all(&existed_parent).unwrap();
        std::fs::create_dir_all(&dne_parent).unwrap();

        std::fs::write(existed_parent.join("backup.json"), "backup content").unwrap();
        std::fs::write(dne_parent.join("dst.json"), "leftover").unwrap();

        // Only lock existed_parent so its rename-back fails with EACCES.
        // Leave dne_parent at default 0o755 so its remove_file succeeds.
        std::fs::set_permissions(&existed_parent, std::fs::Permissions::from_mode(0o555)).unwrap();

        // Vec order matters: rollback walks `.iter().rev()`, so index 1
        // (Existed, locked) is processed first and index 0 (DidNotExist,
        // writable) is processed second.
        let snapshots = vec![
            Snapshot::DidNotExist {
                dst: filesys::File::new(dne_parent.join("dst.json")),
            },
            Snapshot::Existed {
                dst: filesys::File::new(existed_parent.join("dst.json")),
                backup: filesys::File::new(existed_parent.join("backup.json")),
            },
        ];

        rollback(&snapshots).await;

        // Restore permissions BEFORE assertions so tempdir Drop can recurse
        // and clean up even if an assertion fails.
        std::fs::set_permissions(&existed_parent, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert!(
            existed_parent.join("backup.json").exists(),
            "Existed backup file should remain when rename-back into a chmod-555 parent fails"
        );
        assert!(
            !dne_parent.join("dst.json").exists(),
            "DidNotExist dst.json should have been removed by the second rollback step, proving rollback continued past the first failure"
        );
        assert!(
            !existed_parent.join("dst.json").exists(),
            "Existed dst was never populated; rename-back failed so it should still be absent"
        );
    }
}
