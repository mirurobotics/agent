// standard library
use std::fmt::Display;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::SystemTime;

// internal crates
use crate::filesys::dir::Dir;
use crate::filesys::errors::{
    AtomicWriteFileErr, ConvertUTF8Err, CreateSymlinkErr, DeleteFileErr, FileMetadataErr,
    FileSysErr, InvalidFileOverwriteErr, MoveFileErr, OpenFileErr, ParseJSONErr, ReadFileErr,
    UnknownFileNameErr, UnknownParentDirForFileErr, WriteFileErr,
};
use crate::filesys::path::PathExt;
use crate::trace;

// external crates
use atomicwrites::{AllowOverwrite, AtomicFile};
use secrecy::{ExposeSecretMut, SecretBox};
use serde::de::DeserializeOwned;
use tokio::fs::File as TokioFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[allow(unused_imports)]
use tracing::{debug, error, info, warn};

// DEFINITIONS
/// File struct for interacting with files
#[derive(Clone, Debug)]
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
    /// Create a new File instance
    pub fn new<T: Into<PathBuf>>(path: T) -> Self {
        File { path: path.into() }
    }

    /// Return the name of the file
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

    // Create a new Dir instance from the parent directory of the path for this File
    // instance
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

    pub fn parent_exists(&self) -> Result<bool, FileSysErr> {
        // check parent directory exists
        let parent = self.parent()?;
        Ok(parent.exists())
    }

    /// Read the contents of a file
    pub async fn read_bytes(&self) -> Result<Vec<u8>, FileSysErr> {
        self.assert_exists()?;

        // read file
        let mut file = TokioFile::open(self.to_string()).await.map_err(|e| {
            FileSysErr::OpenFileErr(OpenFileErr {
                source: Box::new(e),
                file: self.clone(),
                trace: trace!(),
            })
        })?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).await.map_err(|e| {
            FileSysErr::ReadFileErr(ReadFileErr {
                source: Box::new(e),
                file: self.clone(),
                trace: trace!(),
            })
        })?;
        Ok(buf)
    }

    pub async fn read_secret_bytes(&self) -> Result<SecretBox<Vec<u8>>, FileSysErr> {
        self.assert_exists()?;

        let mut file = TokioFile::open(self.to_string()).await.map_err(|e| {
            FileSysErr::OpenFileErr(OpenFileErr {
                source: Box::new(e),
                file: self.clone(),
                trace: trace!(),
            })
        })?;

        // Get file size to pre-allocate
        let size = self.size().await?;

        // Create SecretBox with pre-allocated Vec
        let mut secret = SecretBox::new(Box::new(Vec::with_capacity(size as usize)));

        // Read directly into the SecretBox
        file.read_to_end(secret.expose_secret_mut())
            .await
            .map_err(|e| {
                FileSysErr::ReadFileErr(ReadFileErr {
                    source: Box::new(e),
                    file: self.clone(),
                    trace: trace!(),
                })
            })?;

        Ok(secret)
    }

    /// Read the contents of a file as a string
    pub async fn read_string(&self) -> Result<String, FileSysErr> {
        let bytes = self.read_bytes().await?;
        let str_ = std::str::from_utf8(&bytes).map_err(|e| {
            FileSysErr::ConvertUTF8Err(ConvertUTF8Err {
                source: Box::new(e),
                trace: trace!(),
            })
        })?;
        Ok(str_.to_string())
    }

    /// Read the contents of a file as json
    pub async fn read_json<T: DeserializeOwned>(&self) -> Result<T, FileSysErr> {
        self.assert_exists()?;

        // read file
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

    fn validate_overwrite(dest: &File, overwrite: bool) -> Result<(), FileSysErr> {
        if !overwrite && dest.exists() {
            return Err(FileSysErr::InvalidFileOverwriteErr(
                InvalidFileOverwriteErr {
                    file: dest.clone(),
                    overwrite,
                    trace: trace!(),
                },
            ));
        }
        Ok(())
    }

    /// Write bytes to a file. Overwrites the file if it exists.
    pub async fn write_bytes(
        &self,
        buf: &[u8],
        overwrite: bool,
        atomic: bool,
    ) -> Result<(), FileSysErr> {
        // ensure parent directory exists
        self.parent()?.create_if_absent().await?;

        // validate overwrite
        File::validate_overwrite(self, overwrite)?;

        if atomic {
            let af = AtomicFile::new(self.to_string(), AllowOverwrite);
            af.write(|f| f.write_all(buf)).map_err(|e| {
                FileSysErr::AtomicWriteFileErr(AtomicWriteFileErr {
                    source: Box::new(e.into()),
                    file: self.clone(),
                    trace: trace!(),
                })
            })?;
        } else {
            let mut file = TokioFile::create(self.to_string()).await.map_err(|e| {
                FileSysErr::OpenFileErr(OpenFileErr {
                    source: Box::new(e),
                    file: self.clone(),
                    trace: trace!(),
                })
            })?;
            file.write_all(buf).await.map_err(|e| {
                FileSysErr::WriteFileErr(WriteFileErr {
                    source: Box::new(e),
                    file: self.clone(),
                    trace: trace!(),
                })
            })?;
        }
        Ok(())
    }

    /// Write a string to a file. Overwrites the file if it exists.
    pub async fn write_string(
        &self,
        s: &str,
        overwrite: bool,
        atomic: bool,
    ) -> Result<(), FileSysErr> {
        self.write_bytes(s.as_bytes(), overwrite, atomic).await
    }

    /// Write a JSON object to a file. Overwrites the file if it exists.
    pub async fn write_json<T: serde::Serialize>(
        &self,
        obj: &T,
        overwrite: bool,
        atomic: bool,
    ) -> Result<(), FileSysErr> {
        let json_bytes = serde_json::to_vec_pretty(obj).map_err(|e| {
            FileSysErr::ParseJSONErr(ParseJSONErr {
                source: Box::new(e),
                file: self.clone(),
                trace: trace!(),
            })
        })?;

        self.write_bytes(&json_bytes, overwrite, atomic).await
    }

    /// Delete a file
    pub async fn delete(&self) -> Result<(), FileSysErr> {
        if !self.exists() {
            return Ok(());
        }
        tokio::fs::remove_file(self.to_string())
            .await
            .map_err(|e| {
                FileSysErr::DeleteFileErr(DeleteFileErr {
                    source: Box::new(e),
                    file: self.clone(),
                    trace: trace!(),
                })
            })?;
        Ok(())
    }

    /// Rename this file to a new file. Overwrites the new file if it exists.
    pub async fn move_to(&self, new_file: &File, overwrite: bool) -> Result<(), FileSysErr> {
        // source file must exist
        self.assert_exists()?;

        // if this file and the new file are the same, nothing needs to be done
        if self.path() == new_file.path() {
            return Ok(());
        }

        File::validate_overwrite(new_file, overwrite)?;

        // ensure the parent directory of the new file exists and create it if not
        new_file.parent()?.create_if_absent().await?;
        if overwrite {
            new_file.delete().await?;
        }

        // move this file to the new file
        tokio::fs::rename(self.to_string(), new_file.to_string())
            .await
            .map_err(|e| {
                FileSysErr::MoveFileErr(MoveFileErr {
                    source: Box::new(e),
                    src_file: self.clone(),
                    dest_file: new_file.clone(),
                    trace: trace!(),
                })
            })?;
        Ok(())
    }

    // Set the file permissions using octal
    // (https://www.redhat.com/sysadmin/linux-file-permissions-explained)
    pub async fn set_permissions(&self, mode: u32) -> Result<(), FileSysErr> {
        self.assert_exists()?;

        // set file permissions
        tokio::fs::set_permissions(self.to_string(), std::fs::Permissions::from_mode(mode))
            .await
            .map_err(|e| {
                FileSysErr::WriteFileErr(WriteFileErr {
                    source: Box::new(e),
                    file: self.clone(),
                    trace: trace!(),
                })
            })?;
        Ok(())
    }

    // overwrites a symlink if it already exists
    pub async fn create_symlink(&self, link: &File, overwrite: bool) -> Result<(), FileSysErr> {
        self.assert_exists()?;
        File::validate_overwrite(link, overwrite)?;
        link.delete().await?;

        // create symlink
        tokio::fs::symlink(self.to_string(), link.to_string())
            .await
            .map_err(|e| {
                FileSysErr::CreateSymlinkErr(CreateSymlinkErr {
                    source: Box::new(e),
                    file: self.clone(),
                    link: link.clone(),
                    trace: trace!(),
                })
            })?;
        Ok(())
    }

    async fn metadata(&self) -> Result<std::fs::Metadata, FileSysErr> {
        self.assert_exists()?;
        tokio::fs::metadata(self.to_string()).await.map_err(|e| {
            FileSysErr::FileMetadataErr(FileMetadataErr {
                file: self.clone(),
                source: Box::new(e),
                trace: trace!(),
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
