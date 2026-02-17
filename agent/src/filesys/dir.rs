// standard library
use std::fmt::Display;
use std::path::PathBuf;

// internal crates
use crate::filesys::{
    errors::{
        CreateDirErr, CreateTmpDirErr, DeleteDirErr, FileSysErr, InvalidDirNameErr, MoveDirErr,
        MoveDirRollbackErr, PathDoesNotExistErr, ReadDirErr, UnknownCurrentDirErr,
        UnknownDirNameErr, UnknownHomeDirErr, UnknownParentDirForDirErr,
    },
    file::File,
    path::PathExt,
    Overwrite,
};
use crate::trace;

// external crates
#[allow(unused_imports)]
use tracing::{debug, error, info, warn};

/// Dir struct for interacting with directories
#[derive(Clone, Debug)]
pub struct Dir {
    path: PathBuf,
}

impl Display for Dir {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.path.to_str().unwrap_or_default())
    }
}

impl PathExt for Dir {
    fn path(&self) -> &PathBuf {
        &self.path
    }
}

impl Dir {
    pub fn new<T: Into<PathBuf>>(path: T) -> Dir {
        Dir { path: path.into() }
    }

    pub fn new_home_dir() -> Result<Dir, FileSysErr> {
        let home_dir = std::env::var("HOME")
            .map_err(|e| {
                FileSysErr::UnknownHomeDirErr(UnknownHomeDirErr {
                    source: Box::new(e),
                    trace: trace!(),
                })
            })
            .map(PathBuf::from)?;
        Ok(Dir { path: home_dir })
    }

    pub fn new_current_dir() -> Result<Dir, FileSysErr> {
        let current_dir = std::env::current_dir().map_err(|e| {
            FileSysErr::UnknownCurrentDirErr(UnknownCurrentDirErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })?;
        Ok(Dir { path: current_dir })
    }

    pub async fn create_temp_dir(prefix: &str) -> Result<Dir, FileSysErr> {
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

    pub fn name(&self) -> Result<&str, FileSysErr> {
        let file_name_os_str = match self.path.file_name() {
            Some(name) => name,
            None => {
                return Err(FileSysErr::UnknownDirNameErr(UnknownDirNameErr {
                    dir: self.clone(),
                    trace: trace!(),
                }));
            }
        };
        match file_name_os_str.to_str() {
            Some(name) => Ok(name),
            None => Err(FileSysErr::UnknownDirNameErr(UnknownDirNameErr {
                dir: self.clone(),
                trace: trace!(),
            })),
        }
    }

    pub fn parent(&self) -> Result<Dir, FileSysErr> {
        let abs_path = self.abs_path()?;
        let parent = abs_path
            .parent()
            .ok_or(FileSysErr::UnknownParentDirForDirErr(
                UnknownParentDirForDirErr {
                    dir: self.clone(),
                    trace: trace!(),
                },
            ))?;
        Ok(Dir::new(parent))
    }

    pub fn is_valid_dir_name(dir_name: &str) -> bool {
        !dir_name.is_empty()
            && !dir_name.contains('/')
            && !dir_name.contains('\0')
            && dir_name.len() <= 255
    }

    pub fn assert_valid_dir_name(dir_name: &str) -> Result<(), FileSysErr> {
        if !Dir::is_valid_dir_name(dir_name) {
            return Err(FileSysErr::InvalidDirNameErr(InvalidDirNameErr {
                name: dir_name.to_string(),
                trace: trace!(),
            }));
        }
        Ok(())
    }

    pub fn subdir<T: Into<PathBuf>>(&self, rel_path: T) -> Dir {
        let rel_path = rel_path.into();
        let rel_path_stripped = rel_path
            .strip_prefix(std::path::MAIN_SEPARATOR.to_string())
            .unwrap_or(&rel_path);
        let mut new_dir = self.path.clone();
        new_dir = new_dir.join(rel_path_stripped);
        Dir::new(new_dir)
    }

    /// Create this directory and any missing parent directories. If the directory
    /// already exists, this is a no-op.
    pub async fn create(&self) -> Result<(), FileSysErr> {
        tokio::fs::create_dir_all(self.path())
            .await
            .map_err(|e| create_dir_err(self.clone(), e))?;
        Ok(())
    }

    /// Alias for [`create`](Self::create).
    pub async fn create_if_absent(&self) -> Result<(), FileSysErr> {
        self.create().await
    }

    /// Delete a directory and all its contents
    pub async fn delete(&self) -> Result<(), FileSysErr> {
        match tokio::fs::remove_dir_all(self.path()).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(delete_dir_err(self.clone(), e)),
        }
    }

    pub fn file(&self, file_name: &str) -> File {
        use std::path::Path;
        let file_name_path = Path::new(file_name);
        let file_name_stripped = file_name_path
            .strip_prefix(std::path::MAIN_SEPARATOR.to_string())
            .unwrap_or(file_name_path);
        let file_path = self.path.join(file_name_stripped);
        File::new(file_path)
    }

    pub async fn subdirs(&self) -> Result<Vec<Dir>, FileSysErr> {
        let mut dirs = Vec::new();
        let mut entries = tokio::fs::read_dir(self.path())
            .await
            .map_err(|e| read_dir_err(self.clone(), e))?;

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| read_dir_err(self.clone(), e))?
        {
            if entry.path().is_dir() {
                dirs.push(Dir::new(entry.path()));
            }
        }
        Ok(dirs)
    }

    pub async fn files(&self) -> Result<Vec<File>, FileSysErr> {
        let mut files = Vec::new();
        let mut entries = tokio::fs::read_dir(self.path())
            .await
            .map_err(|e| read_dir_err(self.clone(), e))?;

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| read_dir_err(self.clone(), e))?
        {
            if entry.path().is_file() {
                files.push(File::new(entry.path()));
            }
        }
        Ok(files)
    }

    pub async fn is_empty(&self) -> Result<bool, FileSysErr> {
        let mut entries = match tokio::fs::read_dir(self.path()).await {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(read_dir_err(self.clone(), e)),
        };
        let has_entry = entries
            .next_entry()
            .await
            .map_err(|e| read_dir_err(self.clone(), e))?;
        Ok(has_entry.is_none())
    }

    pub async fn delete_if_empty_recursive(&self) -> Result<(), FileSysErr> {
        // a recursive implementation would be much simpler but recursion with rust
        // async is not fun and it's still pretty simple to do with a loop

        struct Item {
            dir: Dir,
            seen_before: bool,
        }

        let mut queue = vec![Item {
            dir: self.clone(),
            seen_before: false,
        }];

        while let Some(item) = queue.pop() {
            // attempt to remove the directory (only succeeds if empty).
            match tokio::fs::remove_dir(item.dir.path()).await {
                Ok(()) => continue,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) if Self::is_not_empty_err(&e) => {}
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
            for subdir in item.dir.subdirs().await? {
                queue.push(Item {
                    dir: subdir,
                    seen_before: false,
                });
            }
        }

        // final attempt on self after children have been processed
        match tokio::fs::remove_dir(self.path()).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) if Self::is_not_empty_err(&e) => {}
            Err(e) => return Err(delete_dir_err(self.clone(), e)),
        }
        Ok(())
    }

    fn is_not_empty_err(e: &std::io::Error) -> bool {
        e.kind() == std::io::ErrorKind::DirectoryNotEmpty
    }

    /// Move this directory to a new directory.
    pub async fn move_to(&self, dest_dir: &Dir, overwrite: Overwrite) -> Result<(), FileSysErr> {
        // if same path, nothing to do (but still verify the source exists)
        if self.path() == dest_dir.path() {
            self.assert_exists()?;
            return Ok(());
        }

        // ensure the parent directory of the new directory exists and create it if not
        dest_dir.parent()?.create_if_absent().await?;

        if overwrite == Overwrite::Deny || !dest_dir.exists() {
            self.move_to_no_overwrite(dest_dir).await
        } else {
            self.move_to_with_overwrite(dest_dir).await
        }
    }

    async fn rename_to(&self, dest_dir: &Dir) -> Result<(), FileSysErr> {
        tokio::fs::rename(self.path(), dest_dir.path())
            .await
            .map_err(|e| move_dir_err(self.clone(), dest_dir.clone(), e))?;
        Ok(())
    }

    async fn move_to_no_overwrite(&self, dest_dir: &Dir) -> Result<(), FileSysErr> {
        // TOCTOU note: rename() has no O_EXCL equivalent, so this pre-check is the
        // best we can do for Overwrite::Deny. The race window is unavoidable.
        dest_dir.assert_doesnt_exist()?;
        self.rename_to(dest_dir).await
    }

    async fn move_to_with_overwrite(&self, dest_dir: &Dir) -> Result<(), FileSysErr> {
        // Place the trash directory as a sibling of self (in self's parent) so
        // its path remains valid after self is renamed in step 2.
        let trash_name = format!(".rename_trash_{}", uuid::Uuid::new_v4());
        let trash_dir = self.parent()?.subdir(trash_name);
        trash_dir.delete().await?;

        // step 1: atomically move dest -> trash
        tokio::fs::rename(dest_dir.path(), trash_dir.path())
            .await
            .map_err(|e| move_dir_err(dest_dir.clone(), trash_dir.clone(), e))?;

        // step 2: atomically move self -> dest
        match tokio::fs::rename(self.path(), dest_dir.path()).await {
            Ok(()) => {
                // step 3: clean up trash (best-effort)
                if let Err(e) = trash_dir.delete().await {
                    warn!("failed to delete trash directory: {e}");
                }
                Ok(())
            }
            // rollback: restore dest from trash and report rollback failures explicitly
            Err(e) => {
                match tokio::fs::rename(trash_dir.path(), dest_dir.path()).await {
                    // rollback succeeded — dest is restored, trash no longer exists
                    Ok(()) => Err(move_dir_err(self.clone(), dest_dir.clone(), e)),
                    // rollback failed — trash still holds the original dest contents
                    Err(rollback_err) => {
                        let cleanup_source = trash_dir.delete().await.err().and_then(
                            |cleanup_err| match cleanup_err {
                                FileSysErr::DeleteDirErr(err) => Some(err.source),
                                _ => None,
                            },
                        );
                        Err(FileSysErr::MoveDirRollbackErr(MoveDirRollbackErr {
                            primary_source: Box::new(e),
                            rollback_source: Box::new(rollback_err),
                            cleanup_source,
                            src_dir: self.clone(),
                            dest_dir: dest_dir.clone(),
                            trash_dir,
                            trace: trace!(),
                        }))
                    }
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
