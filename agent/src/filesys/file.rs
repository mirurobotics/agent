// standard crates
use std::fmt::Display;
use std::io;
use std::io::Write;
use std::path::PathBuf;
use std::time::SystemTime;

// internal crates
use crate::filesys::{
    dir::Dir,
    errors::{
        AtomicWriteFileErr, ConvertUTF8Err, CopyFileErr, CreateSymlinkErr, DeleteFileErr,
        FileMetadataErr, FileSysErr, InvalidFileOverwriteErr, MoveFileErr, OpenFileErr,
        ParseJSONErr, PathDoesNotExistErr, PermissionDeniedErr, ReadFileErr, ReadOnlyFilesystemErr,
        UnknownFileNameErr, UnknownParentDirForFileErr, WriteFileErr,
    },
    path::PathExt,
    Atomic, Overwrite, WriteOptions,
};
use crate::trace;

// external crates
use atomicwrites::{AllowOverwrite, AtomicFile, DisallowOverwrite};
use secrecy::{ExposeSecretMut, SecretBox};
use serde::de::DeserializeOwned;
use tokio::fs::File as TokioFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[allow(unused_imports)]
use tracing::{debug, error, info, warn};

/// File struct for interacting with files
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    path: PathBuf,
}

impl Display for File {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.path.to_str().unwrap_or_default())
    }
}

impl PathExt for File {
    fn path(&self) -> &PathBuf {
        &self.path
    }
}

impl File {
    pub fn new<T: Into<PathBuf>>(path: T) -> Self {
        let path: PathBuf = path.into().components().collect();
        File { path }
    }

    pub fn is_absolute(&self) -> bool {
        self.path().is_absolute()
    }

    pub fn name(&self) -> Result<&str, FileSysErr> {
        let file_name_os_str = match self.path.file_name() {
            Some(name) => name,
            None => {
                return Err(FileSysErr::UnknownFileNameErr(UnknownFileNameErr {
                    file: self.clone(),
                    trace: trace!(),
                }));
            }
        };
        match file_name_os_str.to_str() {
            Some(name) => Ok(name),
            None => Err(FileSysErr::UnknownFileNameErr(UnknownFileNameErr {
                file: self.clone(),
                trace: trace!(),
            })),
        }
    }

    pub fn parent(&self) -> Result<Dir, FileSysErr> {
        let parent = self
            .path
            .parent()
            .ok_or(FileSysErr::UnknownParentDirForFileErr(
                UnknownParentDirForFileErr {
                    file: self.clone(),
                    trace: trace!(),
                },
            ))?;
        Ok(Dir::new(parent))
    }

    pub async fn read_bytes(&self) -> Result<Vec<u8>, FileSysErr> {
        // read file
        let mut file = TokioFile::open(self.path())
            .await
            .map_err(|e| File::map_io_err_for_open(e, self))?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).await.map_err(|e| {
            File::map_io_err(e, self, |e| {
                FileSysErr::ReadFileErr(ReadFileErr {
                    source: Box::new(e),
                    file: self.clone(),
                    trace: trace!(),
                })
            })
        })?;
        Ok(buf)
    }

    pub async fn read_secret_bytes(&self) -> Result<SecretBox<Vec<u8>>, FileSysErr> {
        let mut file = TokioFile::open(self.path())
            .await
            .map_err(|e| File::map_io_err_for_open(e, self))?;

        // read directly into the SecretBox
        let size = self.size().await?;
        let mut secret = SecretBox::new(Box::new(Vec::with_capacity(size as usize)));
        file.read_to_end(secret.expose_secret_mut())
            .await
            .map_err(|e| {
                File::map_io_err(e, self, |e| {
                    FileSysErr::ReadFileErr(ReadFileErr {
                        source: Box::new(e),
                        file: self.clone(),
                        trace: trace!(),
                    })
                })
            })?;

        Ok(secret)
    }

    pub async fn read_string(&self) -> Result<String, FileSysErr> {
        let bytes = self.read_bytes().await?;
        String::from_utf8(bytes).map_err(|e| {
            FileSysErr::ConvertUTF8Err(ConvertUTF8Err {
                source: Box::new(e.utf8_error()),
                trace: trace!(),
            })
        })
    }

    pub async fn read_json<T: DeserializeOwned>(&self) -> Result<T, FileSysErr> {
        let bytes = self.read_bytes().await?;
        let obj: T = serde_json::from_slice(&bytes).map_err(|e| {
            FileSysErr::ParseJSONErr(ParseJSONErr {
                source: Box::new(e),
                file: self.clone(),
                trace: trace!(),
            })
        })?;
        Ok(obj)
    }

    pub async fn append_bytes(
        &self,
        buf: &[u8],
        opts: crate::filesys::AppendOptions,
    ) -> Result<(), FileSysErr> {
        self.parent()?.create_if_absent().await?;
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.path())
            .await
            .map_err(|e| File::map_io_err_for_open(e, self))?;
        file.write_all(buf).await.map_err(|e| {
            File::map_io_err(e, self, |e| {
                FileSysErr::WriteFileErr(WriteFileErr {
                    source: Box::new(e),
                    file: self.clone(),
                    trace: trace!(),
                })
            })
        })?;
        if opts.sync == crate::filesys::Sync::Yes {
            file.sync_data().await.map_err(|e| {
                File::map_io_err(e, self, |e| {
                    FileSysErr::WriteFileErr(WriteFileErr {
                        source: Box::new(e),
                        file: self.clone(),
                        trace: trace!(),
                    })
                })
            })?;
        }
        Ok(())
    }

    pub async fn write_bytes(&self, buf: &[u8], opts: WriteOptions) -> Result<(), FileSysErr> {
        // ensure parent directory exists
        self.parent()?.create_if_absent().await?;

        if opts.atomic == Atomic::Yes {
            let af = match opts.overwrite {
                Overwrite::Allow => AtomicFile::new(self.path(), AllowOverwrite),
                Overwrite::Deny => AtomicFile::new(self.path(), DisallowOverwrite),
            };
            let io_err: Result<(), io::Error> =
                af.write(|f| f.write_all(buf)).map_err(|e| e.into());
            io_err.map_err(|e| {
                File::map_io_err(e, self, |e| {
                    if e.kind() == io::ErrorKind::AlreadyExists {
                        FileSysErr::InvalidFileOverwriteErr(InvalidFileOverwriteErr {
                            file: self.clone(),
                            overwrite: opts.overwrite,
                            trace: trace!(),
                        })
                    } else {
                        FileSysErr::AtomicWriteFileErr(AtomicWriteFileErr {
                            source: Box::new(e),
                            file: self.clone(),
                            trace: trace!(),
                        })
                    }
                })
            })?;
        } else {
            let mut file = match opts.overwrite {
                Overwrite::Deny => {
                    tokio::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(self.path())
                        .await
                }
                Overwrite::Allow => TokioFile::create(self.path()).await,
            }
            .map_err(|e| File::map_io_err_for_create(e, self, opts.overwrite))?;
            file.write_all(buf).await.map_err(|e| {
                File::map_io_err(e, self, |e| {
                    FileSysErr::WriteFileErr(WriteFileErr {
                        source: Box::new(e),
                        file: self.clone(),
                        trace: trace!(),
                    })
                })
            })?;
        }
        Ok(())
    }

    pub async fn write_string(&self, s: &str, opts: WriteOptions) -> Result<(), FileSysErr> {
        self.write_bytes(s.as_bytes(), opts).await
    }

    pub async fn write_json<T: serde::Serialize>(
        &self,
        obj: &T,
        opts: WriteOptions,
    ) -> Result<(), FileSysErr> {
        let json_bytes = serde_json::to_vec_pretty(obj).map_err(|e| {
            FileSysErr::ParseJSONErr(ParseJSONErr {
                source: Box::new(e),
                file: self.clone(),
                trace: trace!(),
            })
        })?;

        self.write_bytes(&json_bytes, opts).await
    }

    pub async fn delete(&self) -> Result<(), FileSysErr> {
        match tokio::fs::remove_file(self.path()).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(File::map_io_err(e, self, |e| {
                FileSysErr::DeleteFileErr(DeleteFileErr {
                    source: Box::new(e),
                    file: self.clone(),
                    trace: trace!(),
                })
            })),
        }
    }

    /// Copy this file to a new file. The source is left unchanged.
    /// Honors `Overwrite::Deny` by returning `InvalidFileOverwriteErr`
    /// if `new_file` already exists. A `NotFound` from the underlying
    /// I/O is mapped to `PathDoesNotExistErr` — the same "source missing"
    /// variant `move_to` uses — so callers can pattern-match consistently.
    pub async fn copy_to(&self, new_file: &File, overwrite: Overwrite) -> Result<(), FileSysErr> {
        // if this file and the new file are the same, nothing needs to be done
        // (but still verify the source exists — no I/O to map errors from here)
        if self.path() == new_file.path() {
            self.assert_exists()?;
            return Ok(());
        }

        // TOCTOU note: tokio::fs::copy has no O_EXCL equivalent, so this
        // pre-check is the best we can do for Overwrite::Deny. The race
        // window is unavoidable.
        if overwrite == Overwrite::Deny && new_file.exists() {
            return Err(FileSysErr::InvalidFileOverwriteErr(
                InvalidFileOverwriteErr {
                    file: new_file.clone(),
                    overwrite,
                    trace: trace!(),
                },
            ));
        }

        // ensure the parent directory of the new file exists and create it if not
        new_file.parent()?.create_if_absent().await?;

        tokio::fs::copy(self.path(), new_file.path())
            .await
            .map_err(|e| {
                File::map_io_err(e, new_file, |e| {
                    if e.kind() == io::ErrorKind::NotFound {
                        FileSysErr::PathDoesNotExistErr(PathDoesNotExistErr {
                            path: self.path().clone(),
                            trace: trace!(),
                        })
                    } else {
                        FileSysErr::CopyFileErr(CopyFileErr {
                            source: Box::new(e),
                            src_file: self.clone(),
                            dest_file: new_file.clone(),
                            trace: trace!(),
                        })
                    }
                })
            })?;
        Ok(())
    }

    /// Rename this file to a new file.
    pub async fn move_to(&self, new_file: &File, overwrite: Overwrite) -> Result<(), FileSysErr> {
        // if this file and the new file are the same, nothing needs to be done
        // (but still verify the source exists — no I/O to map errors from here)
        if self.path() == new_file.path() {
            self.assert_exists()?;
            return Ok(());
        }

        // TOCTOU note: rename() has no O_EXCL equivalent, so this pre-check is the
        // best we can do for Overwrite::Deny. The race window is unavoidable.
        if overwrite == Overwrite::Deny && new_file.exists() {
            return Err(FileSysErr::InvalidFileOverwriteErr(
                InvalidFileOverwriteErr {
                    file: new_file.clone(),
                    overwrite,
                    trace: trace!(),
                },
            ));
        }

        // ensure the parent directory of the new file exists and create it if not
        new_file.parent()?.create_if_absent().await?;

        // rename() on Linux atomically replaces the destination file, so no
        // explicit delete is needed for Overwrite::Allow.
        tokio::fs::rename(self.path(), new_file.path())
            .await
            .map_err(|e| {
                File::map_io_err(e, new_file, |e| {
                    if e.kind() == io::ErrorKind::NotFound {
                        FileSysErr::PathDoesNotExistErr(PathDoesNotExistErr {
                            path: self.path().clone(),
                            trace: trace!(),
                        })
                    } else {
                        FileSysErr::MoveFileErr(MoveFileErr {
                            source: Box::new(e),
                            src_file: self.clone(),
                            dest_file: new_file.clone(),
                            trace: trace!(),
                        })
                    }
                })
            })?;
        Ok(())
    }

    // Set the file permissions using octal
    // (https://www.redhat.com/sysadmin/linux-file-permissions-explained)
    pub async fn set_permissions(
        &self,
        permissions: std::fs::Permissions,
    ) -> Result<(), FileSysErr> {
        tokio::fs::set_permissions(self.path(), permissions)
            .await
            .map_err(|e| {
                File::map_io_err(e, self, |e| {
                    if e.kind() == io::ErrorKind::NotFound {
                        FileSysErr::PathDoesNotExistErr(PathDoesNotExistErr {
                            path: self.path().clone(),
                            trace: trace!(),
                        })
                    } else {
                        FileSysErr::WriteFileErr(WriteFileErr {
                            source: Box::new(e),
                            file: self.clone(),
                            trace: trace!(),
                        })
                    }
                })
            })?;
        Ok(())
    }

    pub async fn create_symlink(
        &self,
        link: &File,
        overwrite: Overwrite,
    ) -> Result<(), FileSysErr> {
        // TOCTOU note: symlink() doesn't verify the source exists, so this
        // semantic check cannot be made atomic. Kept as an intentional guard.
        self.assert_exists()?;

        match overwrite {
            Overwrite::Allow => {
                link.delete().await?;
            }
            Overwrite::Deny => { /* let symlink() fail with AlreadyExists below */ }
        }

        // create symlink
        tokio::fs::symlink(self.path(), link.path())
            .await
            .map_err(|e| {
                File::map_io_err(e, link, |e| {
                    if e.kind() == io::ErrorKind::AlreadyExists {
                        FileSysErr::InvalidFileOverwriteErr(InvalidFileOverwriteErr {
                            file: link.clone(),
                            overwrite,
                            trace: trace!(),
                        })
                    } else {
                        FileSysErr::CreateSymlinkErr(CreateSymlinkErr {
                            source: Box::new(e),
                            file: self.clone(),
                            link: link.clone(),
                            trace: trace!(),
                        })
                    }
                })
            })?;
        Ok(())
    }

    async fn metadata(&self) -> Result<std::fs::Metadata, FileSysErr> {
        tokio::fs::metadata(self.path()).await.map_err(|e| {
            File::map_io_err(e, self, |e| {
                if e.kind() == io::ErrorKind::NotFound {
                    FileSysErr::PathDoesNotExistErr(PathDoesNotExistErr {
                        path: self.path().clone(),
                        trace: trace!(),
                    })
                } else {
                    FileSysErr::FileMetadataErr(FileMetadataErr {
                        file: self.clone(),
                        source: Box::new(e),
                        trace: trace!(),
                    })
                }
            })
        })
    }

    pub async fn permissions(&self) -> Result<std::fs::Permissions, FileSysErr> {
        Ok(self.metadata().await?.permissions())
    }

    pub async fn last_modified(&self) -> Result<SystemTime, FileSysErr> {
        Ok(self
            .metadata()
            .await?
            .modified()
            .unwrap_or(SystemTime::now()))
    }

    pub async fn size(&self) -> Result<u64, FileSysErr> {
        Ok(self.metadata().await?.len())
    }

    /// Classify a `tokio::fs` `io::Error` into a `FileSysErr`. EACCES and
    /// EROFS are mapped to friendly variants whose Display strings carry
    /// operator-actionable reason text. Anything else is forwarded to the
    /// `fallback` closure, which produces the operation-specific wrapper
    /// (`AtomicWriteFileErr`, `MoveFileErr`, `CopyFileErr`, etc.).
    ///
    /// Note: EACCES/EROFS are checked BEFORE the fallback. For an
    /// operation like `move_to` whose source is missing AND parent is
    /// unreadable, whichever errno the kernel returns first wins; this
    /// is implementation-defined.
    pub(super) fn map_io_err<F>(err: io::Error, file: &File, fallback: F) -> FileSysErr
    where
        F: FnOnce(io::Error) -> FileSysErr,
    {
        match err.kind() {
            io::ErrorKind::PermissionDenied => {
                FileSysErr::PermissionDeniedErr(PermissionDeniedErr {
                    source: Box::new(err),
                    file: file.clone(),
                    trace: trace!(),
                })
            }
            io::ErrorKind::ReadOnlyFilesystem => {
                FileSysErr::ReadOnlyFilesystemErr(ReadOnlyFilesystemErr {
                    source: Box::new(err),
                    file: file.clone(),
                    trace: trace!(),
                })
            }
            _ => fallback(err),
        }
    }

    fn map_io_err_for_open(e: io::Error, file: &File) -> FileSysErr {
        File::map_io_err(e, file, |e| {
            if e.kind() == io::ErrorKind::NotFound {
                FileSysErr::PathDoesNotExistErr(PathDoesNotExistErr {
                    path: file.path().clone(),
                    trace: trace!(),
                })
            } else {
                FileSysErr::OpenFileErr(OpenFileErr {
                    source: Box::new(e),
                    file: file.clone(),
                    trace: trace!(),
                })
            }
        })
    }

    fn map_io_err_for_create(e: io::Error, file: &File, overwrite: Overwrite) -> FileSysErr {
        File::map_io_err(e, file, |e| {
            if e.kind() == io::ErrorKind::AlreadyExists {
                FileSysErr::InvalidFileOverwriteErr(InvalidFileOverwriteErr {
                    file: file.clone(),
                    overwrite,
                    trace: trace!(),
                })
            } else {
                FileSysErr::OpenFileErr(OpenFileErr {
                    source: Box::new(e),
                    file: file.clone(),
                    trace: trace!(),
                })
            }
        })
    }
}

pub fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            // Allow alphanumeric and some safe characters
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => c,
            // Replace everything else with underscore
            _ => '_',
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_io_err_classifies_permission_denied() {
        let file = File::new(PathBuf::from("/tmp/synthetic.json"));
        let err = io::Error::from(io::ErrorKind::PermissionDenied);
        let result = File::map_io_err(err, &file, |_| {
            panic!("fallback should not run for PermissionDenied")
        });
        assert!(
            matches!(result, FileSysErr::PermissionDeniedErr(_)),
            "expected PermissionDeniedErr, got {result:?}"
        );
        let msg = result.to_string();
        assert!(
            msg.contains("permission denied for path"),
            "expected friendly message, got: {msg}"
        );
        assert!(
            msg.contains("/tmp/synthetic.json"),
            "expected path in message, got: {msg}"
        );
    }

    #[test]
    fn map_io_err_classifies_read_only_filesystem() {
        let file = File::new(PathBuf::from("/tmp/synthetic.json"));
        let err = io::Error::from(io::ErrorKind::ReadOnlyFilesystem);
        let result = File::map_io_err(err, &file, |_| {
            panic!("fallback should not run for ReadOnlyFilesystem")
        });
        assert!(
            matches!(result, FileSysErr::ReadOnlyFilesystemErr(_)),
            "expected ReadOnlyFilesystemErr, got {result:?}"
        );
        let msg = result.to_string();
        assert!(
            msg.contains("read-only mount or denied by a systemd sandbox drop-in"),
            "expected friendly EROFS message, got: {msg}"
        );
    }

    #[test]
    fn map_io_err_falls_through_for_other_kinds() {
        let file = File::new(PathBuf::from("/tmp/synthetic.json"));
        let err = io::Error::from(io::ErrorKind::Other);
        let result = File::map_io_err(err, &file, |e| {
            FileSysErr::AtomicWriteFileErr(AtomicWriteFileErr {
                source: Box::new(e),
                file: file.clone(),
                trace: trace!(),
            })
        });
        assert!(
            matches!(result, FileSysErr::AtomicWriteFileErr(_)),
            "expected fallback AtomicWriteFileErr, got {result:?}"
        );
    }
}
