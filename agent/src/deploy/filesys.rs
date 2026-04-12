// standard crates
use std::collections::HashMap;
use std::path::Component;

// internal crates
use crate::deploy::errors::*;
use crate::filesys::{self, errors::FileSysErr, PathExt, WriteOptions};
use crate::models;
use crate::storage;
use crate::trace;

// external crates
use tracing::{error, info, warn};

pub const BACKUP_FILE_PREFIX: &str = "miru.backup";

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
            trace: trace!(),
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
            trace: trace!(),
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
    let mut seen: HashMap<String, String> = HashMap::new();

    for cfg_inst in cfg_insts {
        let file = filesys::File::new(&cfg_inst.filepath);
        validate_filepath(&file)?;

        let normalized_key = file.path().display().to_string();
        if let Some(first_cfg_inst_id) = seen.insert(normalized_key.clone(), cfg_inst.id.clone()) {
            return Err(DeployErr::DuplicateFilepath(DuplicateFilepathErr {
                filepath: normalized_key,
                cfg_inst_ids: vec![first_cfg_inst_id, cfg_inst.id.clone()],
                trace: trace!(),
            }));
        }
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

fn is_access_denied(kind: std::io::ErrorKind) -> bool {
    matches!(
        kind,
        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem
    )
}

fn map_write_err(cfg_inst: &models::ConfigInstance, err: FileSysErr) -> DeployErr {
    match err {
        FileSysErr::AtomicWriteFileErr(atomic_write_err)
            if is_access_denied(atomic_write_err.source.kind()) =>
        {
            WriteAccessDeniedErr {
                cfg_inst_id: cfg_inst.id.clone(),
                filepath: cfg_inst.filepath.clone(),
                source: atomic_write_err.source,
                trace: trace!(),
            }
            .into()
        }
        _ => err.into(),
    }
}

fn map_snapshot_err(
    cfg_inst: &models::ConfigInstance,
    dest: &filesys::File,
    backup: &filesys::File,
    err: FileSysErr,
) -> DeployErr {
    match err {
        FileSysErr::CopyFileErr(copy_err) if is_access_denied(copy_err.source.kind()) => {
            BackupAccessDeniedErr {
                cfg_inst_id: cfg_inst.id.clone(),
                filepath: dest.path().display().to_string(),
                backup_filepath: backup.path().display().to_string(),
                source: copy_err.source,
                trace: trace!(),
            }
            .into()
        }
        _ => err.into(),
    }
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
        let backup = backup_location(&dest)?;
        let snapshot = snapshot(&dest, &backup)
            .await
            .map_err(|e| map_snapshot_err(cfg_inst, &dest, &backup, e))?;
        snapshots.push(snapshot);

        dest.write_string(&content, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .map_err(|e| map_write_err(cfg_inst, e))?;
    }

    remove_backups(snapshots).await;
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

async fn snapshot(dst: &filesys::File, backup: &filesys::File) -> Result<Snapshot, FileSysErr> {
    // The backup only needs to survive within the same process run for
    // application-level rollback, not across power loss — skip fsync.
    match dst
        .copy_to(backup, filesys::CopyOptions::OVERWRITE)
        .await
    {
        Ok(()) => Ok(Snapshot::Existed {
            dst: dst.clone(),
            backup: backup.clone(),
        }),
        Err(e) => match e {
            FileSysErr::PathDoesNotExistErr(_) => Ok(Snapshot::DidNotExist { dst: dst.clone() }),
            e => Err(e),
        },
    }
}

fn backup_location(dst: &filesys::File) -> Result<filesys::File, FileSysErr> {
    let parent = dst.parent()?;
    let name = dst.name()?;
    let sibling_name = format!("{BACKUP_FILE_PREFIX}.{name}");
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

/// Best-effort housekeeping that removes the backup file from each `Existed` snapshot
/// on the success path. `NotFound` is ignored; other failures are logged at `warn!`
/// level. Never returns an error.
async fn remove_backups(snapshots: &[Snapshot]) {
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

    remove_cfg_insts(&cfg_insts, keeps).await
}

async fn remove_cfg_insts(
    cfg_insts: &[models::ConfigInstance],
    keeps: &[filesys::File],
) -> Result<(), DeployErr> {
    for cfg_inst in cfg_insts {
        let dest = filesys::File::new(&cfg_inst.filepath);
        if keeps.contains(&dest) {
            continue;
        }
        dest.delete().await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    // standard crates
    use std::io;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    // internal crates
    use super::*;
    use crate::filesys;

    // ============================= map_write_err ============================= //

    #[test]
    fn map_write_err_maps_permission_denied_atomic_write_to_write_access_denied() {
        let cfg_inst = models::ConfigInstance {
            id: "cfg_1".to_string(),
            filepath: "/tmp/config.json".to_string(),
            ..Default::default()
        };
        let err = FileSysErr::AtomicWriteFileErr(filesys::errors::AtomicWriteFileErr {
            file: filesys::File::new(&cfg_inst.filepath),
            source: Box::new(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "permission denied",
            )),
            trace: trace!(),
        });

        let actual = map_write_err(&cfg_inst, err);
        assert!(
            matches!(actual, DeployErr::WriteAccessDenied(_)),
            "expected WriteAccessDenied, got {actual:?}"
        );
    }

    #[test]
    fn map_write_err_maps_read_only_fs_atomic_write_to_write_access_denied() {
        let cfg_inst = models::ConfigInstance {
            id: "cfg_2".to_string(),
            filepath: "/tmp/config.json".to_string(),
            ..Default::default()
        };
        let err = FileSysErr::AtomicWriteFileErr(filesys::errors::AtomicWriteFileErr {
            file: filesys::File::new(&cfg_inst.filepath),
            source: Box::new(io::Error::new(
                io::ErrorKind::ReadOnlyFilesystem,
                "read-only filesystem",
            )),
            trace: trace!(),
        });

        let actual = map_write_err(&cfg_inst, err);
        assert!(
            matches!(actual, DeployErr::WriteAccessDenied(_)),
            "expected WriteAccessDenied, got {actual:?}"
        );
    }

    #[test]
    fn map_write_err_keeps_atomic_write_err_for_non_permission_kinds() {
        let cfg_inst = models::ConfigInstance {
            id: "cfg_3".to_string(),
            filepath: "/tmp/config.json".to_string(),
            ..Default::default()
        };
        let err = FileSysErr::AtomicWriteFileErr(filesys::errors::AtomicWriteFileErr {
            file: filesys::File::new(&cfg_inst.filepath),
            source: Box::new(io::Error::new(io::ErrorKind::NotFound, "missing parent")),
            trace: trace!(),
        });

        let actual = map_write_err(&cfg_inst, err);
        match actual {
            DeployErr::FileSysErr(FileSysErr::AtomicWriteFileErr(e)) => {
                assert_eq!(e.source.kind(), io::ErrorKind::NotFound);
            }
            other => panic!("expected FileSysErr(AtomicWriteFileErr), got {other:?}"),
        }
    }

    #[test]
    fn map_write_err_keeps_non_atomic_filesys_errors_unchanged() {
        let cfg_inst = models::ConfigInstance {
            id: "cfg_4".to_string(),
            filepath: "/tmp/config.json".to_string(),
            ..Default::default()
        };
        let err = FileSysErr::CreateTmpDirErr(filesys::errors::CreateTmpDirErr {
            source: Box::new(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "mkdir denied",
            )),
            trace: trace!(),
        });

        let actual = map_write_err(&cfg_inst, err);
        assert!(
            matches!(
                actual,
                DeployErr::FileSysErr(FileSysErr::CreateTmpDirErr(_))
            ),
            "expected FileSysErr(CreateTmpDirErr), got {actual:?}"
        );
    }

    // ============================= map_snapshot_err ============================= //

    #[test]
    fn map_snapshot_err_maps_copy_permission_denied_to_backup_access_denied() {
        let cfg_inst = models::ConfigInstance {
            id: "cfg_5".to_string(),
            filepath: "/tmp/config.json".to_string(),
            ..Default::default()
        };
        let dest = filesys::File::new(&cfg_inst.filepath);
        let backup = filesys::File::new("/tmp/miru.backup.config.json");
        let err = FileSysErr::CopyFileErr(filesys::errors::CopyFileErr {
            source: Box::new(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "permission denied",
            )),
            src_file: dest.clone(),
            dest_file: backup.clone(),
            trace: trace!(),
        });

        let actual = map_snapshot_err(&cfg_inst, &dest, &backup, err);
        match actual {
            DeployErr::BackupAccessDenied(e) => {
                assert_eq!(e.cfg_inst_id, cfg_inst.id);
                assert_eq!(e.filepath, cfg_inst.filepath);
                assert_eq!(e.backup_filepath, backup.path().display().to_string());
                assert_eq!(e.source.kind(), io::ErrorKind::PermissionDenied);
            }
            other => panic!("expected BackupAccessDenied, got {other:?}"),
        }
    }

    #[test]
    fn map_snapshot_err_maps_copy_read_only_fs_to_backup_access_denied() {
        let cfg_inst = models::ConfigInstance {
            id: "cfg_7".to_string(),
            filepath: "/tmp/config.json".to_string(),
            ..Default::default()
        };
        let dest = filesys::File::new(&cfg_inst.filepath);
        let backup = filesys::File::new("/tmp/miru.backup.config.json");
        let err = FileSysErr::CopyFileErr(filesys::errors::CopyFileErr {
            source: Box::new(io::Error::new(
                io::ErrorKind::ReadOnlyFilesystem,
                "read-only filesystem",
            )),
            src_file: dest.clone(),
            dest_file: backup.clone(),
            trace: trace!(),
        });

        let actual = map_snapshot_err(&cfg_inst, &dest, &backup, err);
        assert!(
            matches!(actual, DeployErr::BackupAccessDenied(_)),
            "expected BackupAccessDenied, got {actual:?}"
        );
    }

    #[test]
    fn map_snapshot_err_keeps_copy_err_for_non_permission_kinds() {
        let cfg_inst = models::ConfigInstance {
            id: "cfg_6".to_string(),
            filepath: "/tmp/config.json".to_string(),
            ..Default::default()
        };
        let dest = filesys::File::new(&cfg_inst.filepath);
        let backup = filesys::File::new("/tmp/miru.backup.config.json");
        let err = FileSysErr::CopyFileErr(filesys::errors::CopyFileErr {
            source: Box::new(io::Error::new(io::ErrorKind::NotFound, "missing source")),
            src_file: dest.clone(),
            dest_file: backup.clone(),
            trace: trace!(),
        });

        let actual = map_snapshot_err(&cfg_inst, &dest, &backup, err);
        assert!(
            matches!(actual, DeployErr::FileSysErr(FileSysErr::CopyFileErr(_))),
            "expected FileSysErr(CopyFileErr), got {actual:?}"
        );
    }

    #[test]
    fn map_snapshot_err_keeps_non_copy_filesys_errors_unchanged() {
        let cfg_inst = models::ConfigInstance {
            id: "cfg_8".to_string(),
            filepath: "/tmp/config.json".to_string(),
            ..Default::default()
        };
        let dest = filesys::File::new(&cfg_inst.filepath);
        let backup = filesys::File::new("/tmp/miru.backup.config.json");
        let err = FileSysErr::CreateTmpDirErr(filesys::errors::CreateTmpDirErr {
            source: Box::new(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "mkdir denied",
            )),
            trace: trace!(),
        });

        let actual = map_snapshot_err(&cfg_inst, &dest, &backup, err);
        assert!(
            matches!(
                actual,
                DeployErr::FileSysErr(FileSysErr::CreateTmpDirErr(_))
            ),
            "expected FileSysErr(CreateTmpDirErr), got {actual:?}"
        );
    }

    // ============================= validate_filepath ============================= //

    #[test]
    fn validate_filepath_accepts_clean_absolute_path() {
        let f = filesys::File::new(PathBuf::from("/etc/myapp/config.json"));
        assert!(validate_filepath(&f).is_ok());
    }

    #[test]
    fn validate_filepath_rejects_relative_path() {
        let f = filesys::File::new(PathBuf::from("v1/motion-control.json"));
        assert!(matches!(
            validate_filepath(&f),
            Err(DeployErr::PathNotAllowed(_))
        ));
    }

    #[test]
    fn validate_filepath_rejects_parent_traversal() {
        let f = filesys::File::new(PathBuf::from("/etc/myapp/../passwd"));
        assert!(matches!(
            validate_filepath(&f),
            Err(DeployErr::PathNotAllowed(_))
        ));
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

    #[cfg(unix)]
    #[tokio::test]
    async fn remove_backups_continues_when_delete_fails() {
        let tmp = tempfile::tempdir().unwrap();

        // Writable dir: backup can be deleted
        let writable_dir = tmp.path().join("writable");
        std::fs::create_dir_all(&writable_dir).unwrap();
        std::fs::write(writable_dir.join("dst.json"), "content").unwrap();
        std::fs::write(writable_dir.join("miru.backup.dst.json"), "backup").unwrap();

        // Locked dir: backup cannot be deleted (EACCES)
        let locked_dir = tmp.path().join("locked");
        std::fs::create_dir_all(&locked_dir).unwrap();
        std::fs::write(locked_dir.join("dst.json"), "content").unwrap();
        std::fs::write(locked_dir.join("miru.backup.dst.json"), "backup").unwrap();
        std::fs::set_permissions(&locked_dir, std::fs::Permissions::from_mode(0o555)).unwrap();

        // Vec order matters: remove_backups iterates forward, so index 0
        // (locked) is processed first and fails, index 1 (writable) is
        // processed second. If remove_backups bailed on the first error,
        // the writable backup would still be on disk.
        let snapshots = vec![
            Snapshot::Existed {
                dst: filesys::File::new(locked_dir.join("dst.json")),
                backup: filesys::File::new(locked_dir.join("miru.backup.dst.json")),
            },
            Snapshot::Existed {
                dst: filesys::File::new(writable_dir.join("dst.json")),
                backup: filesys::File::new(writable_dir.join("miru.backup.dst.json")),
            },
        ];

        remove_backups(&snapshots).await;

        // Restore permissions BEFORE assertions so tempdir Drop can recurse
        std::fs::set_permissions(&locked_dir, std::fs::Permissions::from_mode(0o755)).unwrap();

        // Writable backup was successfully deleted
        assert!(
            !writable_dir.join("miru.backup.dst.json").exists(),
            "writable backup should have been deleted"
        );

        // Locked backup still exists — delete failed, error was logged at warn!
        assert!(
            locked_dir.join("miru.backup.dst.json").exists(),
            "locked backup should still exist since delete was blocked by permissions"
        );
    }
}
