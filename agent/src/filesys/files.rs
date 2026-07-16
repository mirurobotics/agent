// standard crates
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::time::SystemTime;

// internal crates
use crate::filesys::{
    dirs,
    errors::*,
    file::{File, Metadata},
    path::PathExt,
    Atomic, CopyOptions, Overwrite, WriteOptions,
};
use crate::trace;

// external crates
use atomicwrites::{AllowOverwrite, AtomicFile, DisallowOverwrite};
use crc32c::crc32c_append;
use secrecy::{ExposeSecretMut, SecretBox};
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use tokio::fs::File as TokioFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[allow(unused_imports)]
use tracing::{debug, error, info, warn};

/// RAII temp file for TESTS. Owns a `tempfile::NamedTempFile` (Drop deletes the
/// file) plus our `File` handle; the file lives exactly as long as this value.
#[cfg(feature = "test")]
#[derive(Debug)]
pub struct TempFile {
    _guard: tempfile::NamedTempFile,
    file: File,
}

#[cfg(feature = "test")]
impl TempFile {
    pub fn file(&self) -> &File {
        &self.file
    }

    /// Owned `File` to move into a longer-lived owner; valid only while `self` lives.
    pub fn to_file(&self) -> File {
        self.file.clone()
    }
}

#[cfg(feature = "test")]
impl std::ops::Deref for TempFile {
    type Target = File;
    fn deref(&self) -> &File {
        &self.file
    }
}

/// Auto-cleaning temp file for tests. Sync; keeps `prefix` for parity with
/// [`crate::filesys::dirs::temp`]. The returned [`TempFile`] deletes the file on
/// drop, so bind it to a named variable that lives as long as the file is needed.
#[cfg(feature = "test")]
pub fn temp(prefix: &str) -> Result<TempFile, FileSysErr> {
    let guard = tempfile::Builder::new()
        .prefix(prefix)
        .tempfile()
        .map_err(|e| {
            FileSysErr::CreateTmpFileErr(CreateTmpFileErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })?;
    let file = File::new(guard.path().to_path_buf());
    Ok(TempFile {
        _guard: guard,
        file,
    })
}

pub async fn read_bytes(file: &File) -> Result<Vec<u8>, FileSysErr> {
    // read file
    let mut f = TokioFile::open(file.path())
        .await
        .map_err(|e| map_io_err_for_open(e, file))?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).await.map_err(|e| {
        FileSysErr::ReadFileErr(ReadFileErr {
            source: Box::new(e),
            file: file.clone(),
            trace: trace!(),
        })
    })?;
    Ok(buf)
}

/// The content hashes of a file, computed in a single streaming pass.
pub struct ContentHashes {
    /// SHA-256 content digest as `sha256:<lowercase-hex>` — the content identity
    /// used for ledger/dedup comparison.
    pub digest: String,
    /// CRC32C (Castagnoli) of the same bytes. Handed to the object store as the
    /// expected checksum so an upload is rejected if the stored bytes differ
    /// from what the scanner measured (GCS enforces this server-side).
    pub crc32c: u32,
}

/// Computes the SHA-256 digest and CRC32C of `file` in a single streaming pass,
/// so both checksums describe exactly the same bytes.
///
/// Streams the file through a fixed 8 KiB buffer, so memory use is constant
/// regardless of file size (the whole file is never materialized). Hashing is
/// CPU-heavy, so callers may wrap this in `tokio::task::spawn_blocking` for very
/// large files to avoid stalling the async runtime.
pub async fn content_hashes(file: &File) -> Result<ContentHashes, FileSysErr> {
    let mut f = TokioFile::open(file.path())
        .await
        .map_err(|e| map_io_err_for_open(e, file))?;

    let mut hasher = Sha256::new();
    let mut crc: u32 = 0;
    let mut buf = [0u8; 8 * 1024];
    loop {
        let n = f.read(&mut buf).await.map_err(|e| {
            FileSysErr::ReadFileErr(ReadFileErr {
                source: Box::new(e),
                file: file.clone(),
                trace: trace!(),
            })
        })?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        crc = crc32c_append(crc, &buf[..n]);
    }

    let digest = hasher.finalize();
    Ok(ContentHashes {
        digest: format!("sha256:{digest:x}"),
        crc32c: crc,
    })
}

/// Computes the SHA-256 content digest of `file`, returned as `sha256:<lowercase-hex>`.
///
/// A thin wrapper over [`content_hashes`] for callers that only need the digest;
/// see it for the streaming/memory characteristics.
pub async fn hash(file: &File) -> Result<String, FileSysErr> {
    Ok(content_hashes(file).await?.digest)
}

pub async fn read_secret_bytes(file: &File) -> Result<SecretBox<Vec<u8>>, FileSysErr> {
    let mut f = TokioFile::open(file.path())
        .await
        .map_err(|e| map_io_err_for_open(e, file))?;

    // read directly into the SecretBox
    let size = size(file).await?;
    let mut secret = SecretBox::new(Box::new(Vec::with_capacity(size as usize)));
    f.read_to_end(secret.expose_secret_mut())
        .await
        .map_err(|e| {
            FileSysErr::ReadFileErr(ReadFileErr {
                source: Box::new(e),
                file: file.clone(),
                trace: trace!(),
            })
        })?;

    Ok(secret)
}

pub async fn read_string(file: &File) -> Result<String, FileSysErr> {
    let bytes = read_bytes(file).await?;
    String::from_utf8(bytes).map_err(|e| {
        FileSysErr::ConvertUTF8Err(ConvertUTF8Err {
            source: Box::new(e.utf8_error()),
            trace: trace!(),
        })
    })
}

pub async fn read_json<T: DeserializeOwned>(file: &File) -> Result<T, FileSysErr> {
    let bytes = read_bytes(file).await?;
    let obj: T = serde_json::from_slice(&bytes).map_err(|e| {
        FileSysErr::ParseJSONErr(ParseJSONErr {
            source: Box::new(e),
            file: file.clone(),
            trace: trace!(),
        })
    })?;
    Ok(obj)
}

pub async fn append_bytes(
    file: &File,
    buf: &[u8],
    opts: crate::filesys::AppendOptions,
) -> Result<(), FileSysErr> {
    dirs::create_if_absent(&file.parent()?).await?;
    let mut f = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file.path())
        .await
        .map_err(|e| map_io_err_for_open(e, file))?;
    f.write_all(buf).await.map_err(|e| {
        FileSysErr::WriteFileErr(WriteFileErr {
            source: Box::new(e),
            file: file.clone(),
            trace: trace!(),
        })
    })?;
    if opts.sync == crate::filesys::Sync::Yes {
        f.sync_data().await.map_err(|e| {
            FileSysErr::WriteFileErr(WriteFileErr {
                source: Box::new(e),
                file: file.clone(),
                trace: trace!(),
            })
        })?;
    }
    Ok(())
}

pub async fn write_bytes(file: &File, buf: &[u8], opts: WriteOptions) -> Result<(), FileSysErr> {
    // ensure parent directory exists
    dirs::create_if_absent(&file.parent()?).await?;

    if opts.atomic == Atomic::Yes {
        let af = match opts.overwrite {
            Overwrite::Allow => AtomicFile::new(file.path(), AllowOverwrite),
            Overwrite::Deny => AtomicFile::new(file.path(), DisallowOverwrite),
        };
        let write_res = match opts.mode {
            Some(m) => {
                let mut open_opts = std::fs::OpenOptions::new();
                open_opts.write(true).create(true).truncate(true).mode(m);
                af.write_with_options(|f| f.write_all(buf), open_opts)
            }
            None => af.write(|f| f.write_all(buf)),
        };
        let io_err: Result<(), std::io::Error> = write_res.map_err(|e| e.into());
        io_err.map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                FileSysErr::InvalidFileOverwriteErr(InvalidFileOverwriteErr {
                    file: file.clone(),
                    overwrite: opts.overwrite,
                    trace: trace!(),
                })
            } else {
                FileSysErr::AtomicWriteFileErr(AtomicWriteFileErr {
                    source: Box::new(e),
                    file: file.clone(),
                    trace: trace!(),
                })
            }
        })?;
    } else {
        let mut f = match opts.overwrite {
            Overwrite::Deny => {
                let mut open_opts = tokio::fs::OpenOptions::new();
                open_opts.write(true).create_new(true);
                if let Some(m) = opts.mode {
                    open_opts.mode(m);
                }
                open_opts.open(file.path()).await
            }
            Overwrite::Allow => {
                let mut open_opts = tokio::fs::OpenOptions::new();
                open_opts.write(true).create(true).truncate(true);
                if let Some(m) = opts.mode {
                    open_opts.mode(m);
                }
                open_opts.open(file.path()).await
            }
        }
        .map_err(|e| map_io_err_for_create(e, file, opts.overwrite))?;
        f.write_all(buf).await.map_err(|e| {
            FileSysErr::WriteFileErr(WriteFileErr {
                source: Box::new(e),
                file: file.clone(),
                trace: trace!(),
            })
        })?;
        f.flush().await.map_err(|e| {
            FileSysErr::WriteFileErr(WriteFileErr {
                source: Box::new(e),
                file: file.clone(),
                trace: trace!(),
            })
        })?;
    }
    Ok(())
}

pub async fn write_string(file: &File, s: &str, opts: WriteOptions) -> Result<(), FileSysErr> {
    write_bytes(file, s.as_bytes(), opts).await
}

/// Test-only convenience: atomically (over)write `contents` to `file`, panicking
/// on error. Collapses the common `write_string(f, s, OVERWRITE_ATOMIC).await
/// .unwrap()` seed pattern to a single line. For non-default write options, call
/// [`write_string`] directly.
#[cfg(feature = "test")]
pub async fn seed(file: &File, contents: &str) {
    write_string(file, contents, WriteOptions::OVERWRITE_ATOMIC)
        .await
        .unwrap();
}

pub async fn write_json<T: serde::Serialize>(
    file: &File,
    obj: &T,
    opts: WriteOptions,
) -> Result<(), FileSysErr> {
    let json_bytes = serde_json::to_vec_pretty(obj).map_err(|e| {
        FileSysErr::ParseJSONErr(ParseJSONErr {
            source: Box::new(e),
            file: file.clone(),
            trace: trace!(),
        })
    })?;

    write_bytes(file, &json_bytes, opts).await
}

pub async fn delete(file: &File) -> Result<(), FileSysErr> {
    match tokio::fs::remove_file(file.path()).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(FileSysErr::DeleteFileErr(DeleteFileErr {
            source: Box::new(e),
            file: file.clone(),
            trace: trace!(),
        })),
    }
}

/// Copy this file to a new file.
pub async fn copy_to(file: &File, dst: &File, opts: CopyOptions) -> Result<(), FileSysErr> {
    if file.path() == dst.path() {
        file.assert_exists()?;
        return Ok(());
    }

    // TOCTOU note: tokio::fs::copy has no O_EXCL equivalent, so this
    // pre-check is the best we can do for Overwrite::Deny. The race
    // window is unavoidable.
    if opts.overwrite == Overwrite::Deny && dst.exists() {
        return Err(FileSysErr::InvalidFileOverwriteErr(
            InvalidFileOverwriteErr {
                file: dst.clone(),
                overwrite: opts.overwrite,
                trace: trace!(),
            },
        ));
    }

    // ensure the parent directory of the new file exists and create it if not
    dirs::create_if_absent(&dst.parent()?).await?;

    tokio::fs::copy(file.path(), dst.path())
        .await
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                FileSysErr::PathDoesNotExistErr(PathDoesNotExistErr {
                    path: file.path().clone(),
                    trace: trace!(),
                })
            } else {
                FileSysErr::CopyFileErr(CopyFileErr {
                    source: Box::new(e),
                    src_file: file.clone(),
                    dest_file: dst.clone(),
                    trace: trace!(),
                })
            }
        })?;

    if opts.sync == crate::filesys::Sync::Yes {
        let f = TokioFile::open(dst.path())
            .await
            .map_err(|e| map_io_err_for_open(e, dst))?;
        f.sync_data().await.map_err(|e| {
            FileSysErr::WriteFileErr(WriteFileErr {
                source: Box::new(e),
                file: dst.clone(),
                trace: trace!(),
            })
        })?;
    }

    Ok(())
}

/// Rename this file to a new file.
pub async fn move_to(file: &File, new_file: &File, overwrite: Overwrite) -> Result<(), FileSysErr> {
    // if this file and the new file are the same, nothing needs to be done
    // (but still verify the source exists — no I/O to map errors from here)
    if file.path() == new_file.path() {
        file.assert_exists()?;
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
    dirs::create_if_absent(&new_file.parent()?).await?;

    // rename() on Linux atomically replaces the destination file, so no
    // explicit delete is needed for Overwrite::Allow.
    tokio::fs::rename(file.path(), new_file.path())
        .await
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                FileSysErr::PathDoesNotExistErr(PathDoesNotExistErr {
                    path: file.path().clone(),
                    trace: trace!(),
                })
            } else {
                FileSysErr::MoveFileErr(MoveFileErr {
                    source: Box::new(e),
                    src_file: file.clone(),
                    dest_file: new_file.clone(),
                    trace: trace!(),
                })
            }
        })?;
    Ok(())
}

// Set the file permissions using octal
// (https://www.redhat.com/sysadmin/linux-file-permissions-explained)
pub async fn set_permissions(
    file: &File,
    permissions: std::fs::Permissions,
) -> Result<(), FileSysErr> {
    tokio::fs::set_permissions(file.path(), permissions)
        .await
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                FileSysErr::PathDoesNotExistErr(PathDoesNotExistErr {
                    path: file.path().clone(),
                    trace: trace!(),
                })
            } else {
                FileSysErr::WriteFileErr(WriteFileErr {
                    source: Box::new(e),
                    file: file.clone(),
                    trace: trace!(),
                })
            }
        })?;
    Ok(())
}

pub async fn create_symlink(
    file: &File,
    link: &File,
    overwrite: Overwrite,
) -> Result<(), FileSysErr> {
    // TOCTOU note: symlink() doesn't verify the source exists, so this
    // semantic check cannot be made atomic. Kept as an intentional guard.
    file.assert_exists()?;

    match overwrite {
        Overwrite::Allow => {
            delete(link).await?;
        }
        Overwrite::Deny => { /* let symlink() fail with AlreadyExists below */ }
    }

    // create symlink
    tokio::fs::symlink(file.path(), link.path())
        .await
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                FileSysErr::InvalidFileOverwriteErr(InvalidFileOverwriteErr {
                    file: link.clone(),
                    overwrite,
                    trace: trace!(),
                })
            } else {
                FileSysErr::CreateSymlinkErr(CreateSymlinkErr {
                    source: Box::new(e),
                    file: file.clone(),
                    link: link.clone(),
                    trace: trace!(),
                })
            }
        })?;
    Ok(())
}

/// Returns the regular files matching a shell-style glob `pattern`.
///
/// A pattern-syntax error (an invalid glob) returns [`FileSysErr::InvalidGlobErr`].
/// Entries that error mid-traversal (e.g. an unreadable directory) are skipped
/// rather than aborting the walk (skip-and-continue). Matching follows symlinks
/// and returns only regular files (per [`Path::is_file`]), so directories are
/// excluded from the results.
pub fn glob(pattern: &str) -> Result<Vec<File>, FileSysErr> {
    let mut matches = Vec::new();

    let paths = glob::glob(pattern).map_err(|e| {
        FileSysErr::InvalidGlobErr(InvalidGlobErr {
            pattern: pattern.to_string(),
            source: Box::new(e),
            trace: trace!(),
        })
    })?;

    for entry in paths {
        let path = match entry {
            Ok(path) => path,
            Err(_) => continue,
        };
        if !path.is_file() {
            continue;
        }
        matches.push(File::new(path));
    }

    Ok(matches)
}

pub async fn metadata(file: &File) -> Result<Metadata, FileSysErr> {
    tokio::fs::metadata(file.path()).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            FileSysErr::PathDoesNotExistErr(PathDoesNotExistErr {
                path: file.path().clone(),
                trace: trace!(),
            })
        } else {
            FileSysErr::FileMetadataErr(FileMetadataErr {
                file: file.clone(),
                source: Box::new(e),
                trace: trace!(),
            })
        }
    })
}

pub async fn permissions(file: &File) -> Result<std::fs::Permissions, FileSysErr> {
    Ok(metadata(file).await?.permissions())
}

pub async fn last_modified(file: &File) -> Result<SystemTime, FileSysErr> {
    Ok(metadata(file)
        .await?
        .modified()
        .unwrap_or(SystemTime::now()))
}

pub async fn size(file: &File) -> Result<u64, FileSysErr> {
    Ok(metadata(file).await?.len())
}

fn map_io_err_for_open(e: std::io::Error, file: &File) -> FileSysErr {
    if e.kind() == std::io::ErrorKind::NotFound {
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
}

fn map_io_err_for_create(e: std::io::Error, file: &File, overwrite: Overwrite) -> FileSysErr {
    if e.kind() == std::io::ErrorKind::AlreadyExists {
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
}

#[cfg(all(test, feature = "test"))]
mod tests {
    // internal crates
    use super::*;

    // ================================ temp ================================= //

    #[tokio::test]
    async fn temp_file_exists_while_guard_alive_and_deletes_on_drop() {
        let tf = temp("testing").unwrap();

        // the file exists while the guard is alive
        let path = tf.path().clone();
        assert!(path.exists());

        // `file()`/`to_file()` return the same underlying path
        assert_eq!(tf.file().path(), &path);
        assert_eq!(tf.to_file().path(), &path);

        // round-trip bytes through the temp file
        write_bytes(tf.file(), b"hello temp", WriteOptions::OVERWRITE_NONATOMIC)
            .await
            .unwrap();
        let bytes = read_bytes(tf.file()).await.unwrap();
        assert_eq!(bytes, b"hello temp");

        // dropping the guard deletes the file
        drop(tf);
        assert!(!path.exists());
    }
}
