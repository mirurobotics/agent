// standard crates
use std::io::Write;
#[cfg(windows)]
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

// internal crates
use crate::cooldown;
use crate::filesys::{
    dirs,
    errors::*,
    file::{File, Metadata},
    path::PathExt,
    Atomic, Overwrite, WriteOptions,
};
use crate::trace;

// external crates
use atomicwrites::{AllowOverwrite, AtomicFile, DisallowOverwrite};
use crc_fast::CrcAlgorithm;
use secrecy::{ExposeSecretMut, SecretBox};
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use tokio::fs::File as TokioFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[allow(unused_imports)]
use tracing::{debug, error, info, warn};
use walkdir::WalkDir;
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
};

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

/// Computes the SHA-256 content digest of `file`, returned as `sha256:<lowercase-hex>`.
///
/// Streams the file through a fixed 8 KiB buffer, so memory use is constant
/// regardless of file size (the whole file is never materialized). Hashing is
/// CPU-heavy, so callers may wrap this in `tokio::task::spawn_blocking` for very
/// large files to avoid stalling the async runtime.
pub async fn sha256(file: &File) -> Result<String, FileSysErr> {
    let mut hasher = Sha256::new();
    read_chunks(file, |chunk| hasher.update(chunk)).await?;
    let digest = hasher.finalize();
    Ok(format!("sha256:{digest:x}"))
}

/// Computes `file`'s [`sha256`] digest and its CRC32C (Castagnoli) checksum in
/// one read.
pub async fn sha256_and_crc32c(file: &File) -> Result<(String, u32), FileSysErr> {
    let mut hasher = Sha256::new();
    let mut crc32c = crc_fast::Digest::new(CrcAlgorithm::Crc32Iscsi);
    read_chunks(file, |chunk| {
        hasher.update(chunk);
        crc32c.update(chunk);
    })
    .await?;
    let digest = hasher.finalize();
    // CRC-32 values fit in the low 32 bits of crc-fast's u64
    Ok((format!("sha256:{digest:x}"), crc32c.finalize() as u32))
}

/// Reads `file` through a fixed 8 KiB buffer, passing each chunk to `on_chunk`.
/// The file is opened with [`open_regular_file`].
async fn read_chunks(file: &File, mut on_chunk: impl FnMut(&[u8])) -> Result<(), FileSysErr> {
    let mut f = open_regular_file(file)
        .await
        .map_err(|e| map_io_err_for_open(e, file))?;

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
        on_chunk(&buf[..n]);
    }
    Ok(())
}

/// Opens `file` for reading and refuses anything but a regular file, including
/// a symlink, so a link planted in a watched directory is never read through.
/// On Windows it also refuses every other reparse point.
pub async fn open_regular_file(file: &File) -> std::io::Result<TokioFile> {
    let mut opts = tokio::fs::OpenOptions::new();
    opts.read(true);
    // O_NONBLOCK keeps a FIFO from blocking the open until it's refused
    #[cfg(unix)]
    opts.custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK);
    // opens a link itself rather than its target, so the check below refuses it
    #[cfg(windows)]
    opts.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    let f = opts.open(file.path()).await?;
    let meta = f.metadata().await?;
    // on Windows the flag also skips a placeholder's filter, so its stub would be read
    if !meta.is_file() || is_reparse_point(&meta) {
        let kind = std::io::ErrorKind::InvalidInput;
        return Err(std::io::Error::new(kind, "not a regular file"));
    }
    Ok(f)
}

/// Whether `meta` is a reparse point, such as a link or a cloud placeholder.
#[cfg(windows)]
fn is_reparse_point(meta: &std::fs::Metadata) -> bool {
    meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(unix)]
fn is_reparse_point(_meta: &std::fs::Metadata) -> bool {
    false
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

/// Waits between retries of a read that failed for a reason other than
/// "missing" or "unparseable" (e.g. a transient EIO).
const READ_RETRY_DELAYS: [Duration; 3] = [
    Duration::from_millis(50),
    Duration::from_millis(250),
    Duration::from_millis(1000),
];

/// [`read_json`], retrying errors other than "missing" and "unparseable",
/// which a retry won't change, after each of [`READ_RETRY_DELAYS`].
pub async fn read_json_with_retry<T: DeserializeOwned>(file: &File) -> Result<T, FileSysErr> {
    let is_retryable = |err: &FileSysErr| {
        !matches!(
            err,
            FileSysErr::PathDoesNotExistErr(_) | FileSysErr::ParseJSONErr(_)
        )
    };
    cooldown::retry(
        |n| READ_RETRY_DELAYS.get(n as usize).copied(),
        is_retryable,
        tokio::time::sleep,
        move || read_json(file),
    )
    .await
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

// File mode bits are a Unix concept: on Windows they carry no meaning and
// permissions are governed by NTFS ACLs inherited from the parent directory
// (set up by the installer). These two helpers localize that platform
// difference so the write paths stay platform-agnostic.

/// A preconfigured [`std::fs::OpenOptions`] carrying `mode` for creation-time
/// permissions, or `None` when no mode was requested (always `None` on
/// Windows). Used by the atomic write path.
#[cfg(unix)]
fn mode_open_options(mode: Option<u32>) -> Option<std::fs::OpenOptions> {
    use std::os::unix::fs::OpenOptionsExt;
    mode.map(|m| {
        let mut open_opts = std::fs::OpenOptions::new();
        open_opts.write(true).create(true).truncate(true).mode(m);
        open_opts
    })
}

#[cfg(windows)]
fn mode_open_options(_mode: Option<u32>) -> Option<std::fs::OpenOptions> {
    None
}

/// Apply creation-time `mode` bits to `open_opts`; a no-op on Windows. Used by
/// the non-atomic write path.
#[cfg(unix)]
fn apply_mode(open_opts: &mut tokio::fs::OpenOptions, mode: Option<u32>) {
    // tokio's OpenOptions exposes `mode` as an inherent method under cfg(unix);
    // no OpenOptionsExt import needed here (unlike std::fs::OpenOptions above).
    if let Some(m) = mode {
        open_opts.mode(m);
    }
}

#[cfg(windows)]
fn apply_mode(_open_opts: &mut tokio::fs::OpenOptions, _mode: Option<u32>) {}

pub async fn write_bytes(file: &File, buf: &[u8], opts: WriteOptions) -> Result<(), FileSysErr> {
    // ensure parent directory exists
    dirs::create_if_absent(&file.parent()?).await?;

    if opts.atomic == Atomic::Yes {
        write_bytes_atomic(file, buf, opts)
    } else {
        write_bytes_direct(file, buf, opts).await
    }
}

fn write_bytes_atomic(file: &File, buf: &[u8], opts: WriteOptions) -> Result<(), FileSysErr> {
    let af = match opts.overwrite {
        Overwrite::Allow => AtomicFile::new(file.path(), AllowOverwrite),
        Overwrite::Deny => AtomicFile::new(file.path(), DisallowOverwrite),
    };
    let write_res = match mode_open_options(opts.mode) {
        Some(open_opts) => af.write_with_options(|f| f.write_all(buf), open_opts),
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
    })
}

async fn write_bytes_direct(file: &File, buf: &[u8], opts: WriteOptions) -> Result<(), FileSysErr> {
    let mut open_opts = tokio::fs::OpenOptions::new();
    match opts.overwrite {
        Overwrite::Deny => open_opts.write(true).create_new(true),
        Overwrite::Allow => open_opts.write(true).create(true).truncate(true),
    };
    apply_mode(&mut open_opts, opts.mode);
    let mut f = open_opts
        .open(file.path())
        .await
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
    })
}

pub async fn write_string(file: &File, s: &str, opts: WriteOptions) -> Result<(), FileSysErr> {
    write_bytes(file, s.as_bytes(), opts).await
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

/// Delete `file`, logging a failure instead of returning it. A file that is
/// already gone is fine.
pub async fn delete_or_warn(file: &File) {
    if let Err(e) = delete(file).await {
        warn!("failed to delete {file}: {e}");
    }
}

/// Copy `file` to `dst`. Permissions follow `tokio::fs::copy` (Unix mode bits
/// and Windows attributes).
pub async fn copy_to(file: &File, dst: &File, overwrite: Overwrite) -> Result<(), FileSysErr> {
    if file.path() == dst.path() {
        file.assert_exists()?;
        return Ok(());
    }

    // TOCTOU note: tokio::fs::copy has no O_EXCL equivalent, so this
    // pre-check is the best we can do for Overwrite::Deny.
    if overwrite == Overwrite::Deny && dst.exists() {
        return Err(FileSysErr::InvalidFileOverwriteErr(
            InvalidFileOverwriteErr {
                file: dst.clone(),
                overwrite,
                trace: trace!(),
            },
        ));
    }

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

/// Returns the regular files matching a shell-style glob `pattern`, sorted by path.
///
/// A pattern-syntax error (an invalid glob) returns [`FileSysErr::InvalidGlobErr`].
/// The walk starts at the pattern's literal prefix (the directories before the
/// first wildcard) and enters only directories that can hold a match. It never
/// follows a symlink below the prefix: symlinked files are skipped and
/// symlinked directories aren't entered. On Windows, any other reparse point
/// below the prefix, such as a cloud placeholder, is skipped too. A directory
/// swapped for a link after it's listed isn't caught here. Entries that error
/// mid-walk (e.g. an unreadable directory) are skipped rather than aborting the
/// walk.
pub fn glob(pattern: &str) -> Result<Vec<File>, FileSysErr> {
    let parsed = ParsedGlob::new(pattern)?;
    let walk_from = match parsed.root.as_os_str().is_empty() {
        true => Path::new("."),
        false => parsed.root.as_path(),
    };
    let mut walk = WalkDir::new(walk_from).min_depth(1).sort_by_file_name();
    if let Some(max_depth) = parsed.max_depth {
        walk = walk.max_depth(max_depth);
    }
    let options = glob::MatchOptions {
        require_literal_separator: true,
        ..glob::MatchOptions::new()
    };

    let mut matches = Vec::new();
    let entries = walk
        .into_iter()
        .filter_entry(|entry| !is_skipped_reparse_point(entry) && parsed.may_hold_match(entry));
    for entry in entries.flatten() {
        if entry.path_is_symlink() {
            debug!("glob: skipping symlink {}", entry.path().display());
            continue;
        }
        let Ok(relative) = entry.path().strip_prefix(walk_from) else {
            continue;
        };
        if entry.file_type().is_file() && parsed.below.matches_path_with(relative, options) {
            matches.push(File::new(parsed.root.join(relative)));
        }
    }
    Ok(matches)
}

/// Checks that `pattern` is a glob [`glob`] accepts, without walking the
/// filesystem. An invalid pattern returns [`FileSysErr::InvalidGlobErr`].
pub fn validate_glob(pattern: &str) -> Result<(), FileSysErr> {
    ParsedGlob::new(pattern).map(drop)
}

/// A glob pattern split at its literal prefix, where a walk starts.
struct ParsedGlob {
    root: PathBuf,
    /// The pattern below `root`, matched against each file's relative path.
    below: glob::Pattern,
    /// One pattern per directory level below `root`, up to the first `**`.
    level_patterns: Vec<glob::Pattern>,
    /// How deep a match can lie below `root`; any depth once there's a `**`.
    max_depth: Option<usize>,
}

impl ParsedGlob {
    fn new(pattern: &str) -> Result<Self, FileSysErr> {
        let invalid = |e: glob::PatternError| {
            FileSysErr::InvalidGlobErr(InvalidGlobErr {
                pattern: pattern.to_string(),
                source: Box::new(e),
                trace: trace!(),
            })
        };
        let components: Vec<_> = Path::new(pattern).components().collect();
        let names: Vec<_> = components
            .iter()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect();
        // the last component names the files, so it always stays below the prefix
        let literal = names
            .iter()
            .take(names.len().saturating_sub(1))
            .take_while(|name| !name.contains(['*', '?', '[']))
            .count();
        let below: PathBuf = components[literal..].iter().collect();
        let below = glob::Pattern::new(&below.to_string_lossy());
        let dir_names = &names[literal..names.len().saturating_sub(1).max(literal)];
        let is_recursive = names[literal..].iter().any(|name| name == "**");
        Ok(Self {
            root: components[..literal].iter().collect(),
            below: below.map_err(invalid)?,
            level_patterns: dir_names
                .iter()
                .take_while(|name| *name != "**")
                .map(|name| glob::Pattern::new(name).map_err(invalid))
                .collect::<Result<_, _>>()?,
            max_depth: (!is_recursive).then_some(dir_names.len() + 1),
        })
    }

    /// Whether the walk should keep `entry`: a directory is entered only if
    /// its name matches its level's pattern.
    fn may_hold_match(&self, entry: &walkdir::DirEntry) -> bool {
        if !entry.file_type().is_dir() {
            return true;
        }
        let level = entry.depth().checked_sub(1);
        level
            .and_then(|level| self.level_patterns.get(level))
            .is_none_or(|dir| dir.matches(&entry.file_name().to_string_lossy()))
    }
}

/// Whether the glob leaves out `entry` as a reparse point below the prefix,
/// logging it like a skipped symlink.
#[cfg(windows)]
fn is_skipped_reparse_point(entry: &walkdir::DirEntry) -> bool {
    // the listing already holds each entry's attributes, so this needs no
    // extra system call
    let is_skipped =
        entry.depth() > 0 && entry.metadata().is_ok_and(|meta| is_reparse_point(&meta));
    if is_skipped {
        debug!("glob: skipping reparse point {}", entry.path().display());
    }
    is_skipped
}

#[cfg(unix)]
fn is_skipped_reparse_point(_entry: &walkdir::DirEntry) -> bool {
    false
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

#[cfg(test)]
mod tests {
    // internal crates
    use super::*;
    use crate::test_utils::filesys::files::temp;

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
