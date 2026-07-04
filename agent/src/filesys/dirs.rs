// standard crates
use std::path::PathBuf;

// internal crates
use crate::filesys::dir::Dir;
use crate::filesys::errors::{
    CreateDirErr, CreateTmpDirErr, DeleteDirErr, DirMetadataErr, FileSysErr, MoveDirErr,
    MoveDirRollbackErr, PathDoesNotExistErr, ReadDirErr, SetDirPermissionsErr, UnknownCurrentDirErr,
    UnknownHomeDirErr,
};
use crate::filesys::file::File;
use crate::filesys::path::PathExt;
use crate::filesys::Overwrite;
use crate::trace;

// external crates
#[allow(unused_imports)]
use tracing::{debug, error, info, warn};

pub fn home() -> Result<Dir, FileSysErr> {
    let home_dir = std::env::var("HOME")
        .map_err(|e| {
            FileSysErr::UnknownHomeDirErr(UnknownHomeDirErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })
        .map(PathBuf::from)?;
    Ok(Dir::new(home_dir))
}

pub fn current() -> Result<Dir, FileSysErr> {
    let current_dir = std::env::current_dir().map_err(|e| {
        FileSysErr::UnknownCurrentDirErr(UnknownCurrentDirErr {
            source: Box::new(e),
            trace: trace!(),
        })
    })?;
    Ok(Dir::new(current_dir))
}

pub async fn create_temp(prefix: &str) -> Result<Dir, FileSysErr> {
    let temp_dir = tempfile::Builder::new()
        .prefix(prefix)
        .tempdir()
        .map_err(|e| {
            FileSysErr::CreateTmpDirErr(CreateTmpDirErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })?;
    Ok(Dir::new(temp_dir.keep()))
}

/// Create this directory and any missing parent directories. If the directory
/// already exists, this is a no-op.
pub async fn create(dir: &Dir) -> Result<(), FileSysErr> {
    tokio::fs::create_dir_all(dir.path())
        .await
        .map_err(|e| create_dir_err(dir.clone(), e))?;
    Ok(())
}

/// Alias for [`create`].
pub async fn create_if_absent(dir: &Dir) -> Result<(), FileSysErr> {
    create(dir).await
}

/// Delete a directory and all its contents
pub async fn delete(dir: &Dir) -> Result<(), FileSysErr> {
    match tokio::fs::remove_dir_all(dir.path()).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(delete_dir_err(dir.clone(), e)),
    }
}

pub async fn subdirs(dir: &Dir) -> Result<Vec<Dir>, FileSysErr> {
    let mut dirs = Vec::new();
    let mut entries = tokio::fs::read_dir(dir.path())
        .await
        .map_err(|e| read_dir_err(dir.clone(), e))?;

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| read_dir_err(dir.clone(), e))?
    {
        if entry.path().is_dir() {
            dirs.push(Dir::new(entry.path()));
        }
    }
    Ok(dirs)
}

pub async fn files(dir: &Dir) -> Result<Vec<File>, FileSysErr> {
    let mut files = Vec::new();
    let mut entries = tokio::fs::read_dir(dir.path())
        .await
        .map_err(|e| read_dir_err(dir.clone(), e))?;

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| read_dir_err(dir.clone(), e))?
    {
        if entry.path().is_file() {
            files.push(File::new(entry.path()));
        }
    }
    Ok(files)
}

pub async fn is_empty(dir: &Dir) -> Result<bool, FileSysErr> {
    let mut entries = match tokio::fs::read_dir(dir.path()).await {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(read_dir_err(dir.clone(), e)),
    };
    let has_entry = entries
        .next_entry()
        .await
        .map_err(|e| read_dir_err(dir.clone(), e))?;
    Ok(has_entry.is_none())
}

pub async fn delete_if_empty_recursive(dir: &Dir) -> Result<(), FileSysErr> {
    // a recursive implementation would be much simpler but recursion with rust
    // async is not fun and it's still pretty simple to do with a loop

    struct Item {
        dir: Dir,
        seen_before: bool,
    }

    let mut queue = vec![Item {
        dir: dir.clone(),
        seen_before: false,
    }];

    while let Some(item) = queue.pop() {
        // attempt to remove the directory (only succeeds if empty).
        match tokio::fs::remove_dir(item.dir.path()).await {
            Ok(()) => continue,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) if is_not_empty_err(&e) => {}
            Err(e) => return Err(delete_dir_err(item.dir.clone(), e)),
        }

        // not empty, but we've already seen it — skip
        if item.seen_before {
            continue;
        }

        // we add the parent first so that if its children are deleted and leave it
        // empty, it will be deleted
        queue.push(Item {
            dir: item.dir.clone(),
            seen_before: true,
        });
        for subdir in subdirs(&item.dir).await? {
            queue.push(Item {
                dir: subdir,
                seen_before: false,
            });
        }
    }

    // final attempt on self after children have been processed
    match tokio::fs::remove_dir(dir.path()).await {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) if is_not_empty_err(&e) => {}
        Err(e) => return Err(delete_dir_err(dir.clone(), e)),
    }
    Ok(())
}

fn is_not_empty_err(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::DirectoryNotEmpty
}

/// Set the directory permissions using octal
/// (https://www.redhat.com/sysadmin/linux-file-permissions-explained)
pub async fn set_permissions(
    dir: &Dir,
    permissions: std::fs::Permissions,
) -> Result<(), FileSysErr> {
    tokio::fs::set_permissions(dir.path(), permissions)
        .await
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                FileSysErr::PathDoesNotExistErr(PathDoesNotExistErr {
                    path: dir.path().clone(),
                    trace: trace!(),
                })
            } else {
                FileSysErr::SetDirPermissionsErr(SetDirPermissionsErr {
                    source: Box::new(e),
                    dir: dir.clone(),
                    trace: trace!(),
                })
            }
        })?;
    Ok(())
}

pub async fn permissions(dir: &Dir) -> Result<std::fs::Permissions, FileSysErr> {
    Ok(metadata(dir).await?.permissions())
}

async fn metadata(dir: &Dir) -> Result<std::fs::Metadata, FileSysErr> {
    tokio::fs::metadata(dir.path()).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            FileSysErr::PathDoesNotExistErr(PathDoesNotExistErr {
                path: dir.path().clone(),
                trace: trace!(),
            })
        } else {
            FileSysErr::DirMetadataErr(DirMetadataErr {
                dir: dir.clone(),
                source: Box::new(e),
                trace: trace!(),
            })
        }
    })
}

/// Move this directory to a new directory.
pub async fn move_to(dir: &Dir, dest_dir: &Dir, overwrite: Overwrite) -> Result<(), FileSysErr> {
    // if same path, nothing to do (but still verify the source exists)
    if dir.path() == dest_dir.path() {
        dir.assert_exists()?;
        return Ok(());
    }

    // ensure the parent directory of the new directory exists and create it if not
    create_if_absent(&dest_dir.parent()?).await?;

    if overwrite == Overwrite::Deny || !dest_dir.exists() {
        move_to_no_overwrite(dir, dest_dir).await
    } else {
        move_to_with_overwrite(dir, dest_dir).await
    }
}

async fn rename_to(dir: &Dir, dest_dir: &Dir) -> Result<(), FileSysErr> {
    tokio::fs::rename(dir.path(), dest_dir.path())
        .await
        .map_err(|e| move_dir_err(dir.clone(), dest_dir.clone(), e))?;
    Ok(())
}

async fn move_to_no_overwrite(dir: &Dir, dest_dir: &Dir) -> Result<(), FileSysErr> {
    // TOCTOU note: rename() has no O_EXCL equivalent, so this pre-check is the
    // best we can do for Overwrite::Deny. The race window is unavoidable.
    dest_dir.assert_doesnt_exist()?;
    rename_to(dir, dest_dir).await
}

async fn move_to_with_overwrite(dir: &Dir, dest_dir: &Dir) -> Result<(), FileSysErr> {
    // Place the trash directory as a sibling of self (in self's parent) so
    // its path remains valid after self is renamed in step 2.
    let trash_name = format!(".rename_trash_{}", uuid::Uuid::new_v4());
    let trash_dir = dir.parent()?.subdir(trash_name);
    delete(&trash_dir).await?;

    // step 1: atomically move dest -> trash
    tokio::fs::rename(dest_dir.path(), trash_dir.path())
        .await
        .map_err(|e| move_dir_err(dest_dir.clone(), trash_dir.clone(), e))?;

    // step 2: atomically move self -> dest
    match tokio::fs::rename(dir.path(), dest_dir.path()).await {
        Ok(()) => {
            // step 3: clean up trash (best-effort)
            if let Err(e) = delete(&trash_dir).await {
                debug_assert!(false, "failed to delete trash directory: {e}");
                warn!("failed to delete trash directory: {e}");
            }
            Ok(())
        }
        // rollback: restore dest from trash and report rollback failures explicitly
        Err(e) => {
            match tokio::fs::rename(trash_dir.path(), dest_dir.path()).await {
                // rollback succeeded — dest is restored, trash no longer exists
                Ok(()) => Err(move_dir_err(dir.clone(), dest_dir.clone(), e)),
                // rollback failed — trash still holds the original dest contents
                Err(rollback_err) => {
                    let cleanup_source = delete(&trash_dir).await.err().and_then(
                        |cleanup_err| match cleanup_err {
                            FileSysErr::DeleteDirErr(err) => Some(err.source),
                            _ => None,
                        },
                    );
                    Err(FileSysErr::MoveDirRollbackErr(MoveDirRollbackErr {
                        primary_source: Box::new(e),
                        rollback_source: Box::new(rollback_err),
                        cleanup_source,
                        src_dir: dir.clone(),
                        dest_dir: dest_dir.clone(),
                        trash_dir,
                        trace: trace!(),
                    }))
                }
            }
        }
    }
}

fn read_dir_err(dir: Dir, e: std::io::Error) -> FileSysErr {
    FileSysErr::ReadDirErr(ReadDirErr {
        source: Box::new(e),
        dir,
        trace: trace!(),
    })
}

fn create_dir_err(dir: Dir, e: std::io::Error) -> FileSysErr {
    FileSysErr::CreateDirErr(CreateDirErr {
        source: Box::new(e),
        dir,
        trace: trace!(),
    })
}

fn delete_dir_err(dir: Dir, e: std::io::Error) -> FileSysErr {
    FileSysErr::DeleteDirErr(DeleteDirErr {
        source: Box::new(e),
        dir,
        trace: trace!(),
    })
}

fn move_dir_err(src_dir: Dir, dest_dir: Dir, e: std::io::Error) -> FileSysErr {
    if e.kind() == std::io::ErrorKind::NotFound {
        FileSysErr::PathDoesNotExistErr(PathDoesNotExistErr {
            path: src_dir.path().clone(),
            trace: trace!(),
        })
    } else {
        FileSysErr::MoveDirErr(MoveDirErr {
            source: Box::new(e),
            src_dir,
            dest_dir,
            trace: trace!(),
        })
    }
}
