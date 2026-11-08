// standard crates
use std::ffi::{OsStr, OsString};
use std::fs::FileType;
use std::io::Write;
#[cfg(windows)]
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

// internal crates
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
    if !meta.is_file() || is_reparse_point(&meta) {
        let kind = std::io::ErrorKind::InvalidInput;
        return Err(std::io::Error::new(kind, "not a regular file"));
    }
    Ok(f)
}

/// Whether `meta` is a reparse point, such as a link or a cloud placeholder. A
/// no-follow open skips the placeholder's filter and would read its stub.
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
/// first wildcard) and matches each level below it against that level's
/// pattern component, so it only enters directories that can hold a match. It
/// never follows a symlink below the prefix: symlinked files and directories
/// are skipped, and a `..` below the prefix matches nothing, so a match can't
/// resolve outside the prefix. Directories that error mid-walk (e.g.
/// unreadable ones) are skipped rather than aborting the walk.
pub fn glob(pattern: &str) -> Result<Vec<File>, FileSysErr> {
    let (root, components) = parse_glob(pattern)?;
    let mut walk = GlobWalk {
        todo: vec![(root, components.as_slice())],
        matches: Vec::new(),
    };
    while let Some((dir, components)) = walk.todo.pop() {
        let Some(first) = components.first() else {
            continue;
        };
        for (path, file_type) in glob_entries(&dir, first) {
            walk.visit(path, file_type, components);
        }
    }
    let mut matches = walk.matches;
    matches.sort();
    // two `**` components can reach one file by two routes
    matches.dedup();
    Ok(matches.into_iter().map(File::new).collect())
}

/// Checks that `pattern` is a glob [`glob`] accepts, without walking the
/// filesystem. An invalid pattern returns [`FileSysErr::InvalidGlobErr`].
pub fn validate_glob(pattern: &str) -> Result<(), FileSysErr> {
    parse_glob(pattern).map(drop)
}

const GLOB_META: [char; 3] = ['*', '?', '['];

/// One path component of a glob pattern.
enum GlobComponent {
    /// A name with no wildcards, matched exactly.
    Literal(OsString),
    /// A name pattern such as `*.log`.
    Wildcard(glob::Pattern),
    /// `**`: zero or more directories.
    AnyDirs,
}

impl GlobComponent {
    fn parse(name: &OsStr) -> Result<Self, glob::PatternError> {
        let text = name.to_string_lossy();
        if text == "**" {
            return Ok(Self::AnyDirs);
        }
        if !text.contains(GLOB_META) {
            return Ok(Self::Literal(name.to_os_string()));
        }
        glob::Pattern::new(&text).map(Self::Wildcard)
    }

    fn matches(&self, name: &OsStr) -> bool {
        match self {
            Self::Literal(literal) => name == literal,
            Self::Wildcard(pattern) => pattern.matches_path(Path::new(name)),
            Self::AnyDirs => true,
        }
    }
}

/// Splits `pattern` into its literal prefix, where a walk starts, and its
/// components below that. The last component always stays below the prefix,
/// since it names the files to match.
fn parse_glob(pattern: &str) -> Result<(PathBuf, Vec<GlobComponent>), FileSysErr> {
    let components: Vec<_> = Path::new(pattern).components().collect();
    let mut parsed = components
        .iter()
        .map(|c| GlobComponent::parse(c.as_os_str()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| {
            FileSysErr::InvalidGlobErr(InvalidGlobErr {
                pattern: pattern.to_string(),
                source: Box::new(e),
                trace: trace!(),
            })
        })?;
    let literal = parsed
        .iter()
        .take(parsed.len().saturating_sub(1))
        .take_while(|c| matches!(c, GlobComponent::Literal(_)))
        .count();
    let root = components[..literal].iter().collect();
    Ok((root, parsed.split_off(literal)))
}

/// A glob walk's state. It keeps its own work list rather than recursing, so a
/// deep tree can't overflow the stack.
struct GlobWalk<'p> {
    /// Directories still to read, each with the components left for its entries.
    todo: Vec<(PathBuf, &'p [GlobComponent])>,
    matches: Vec<PathBuf>,
}

impl<'p> GlobWalk<'p> {
    /// Handles `path`, which matches the first of `components`: keeps it if
    /// it's a regular file that matches them all, or queues it if it's a
    /// directory with more to match.
    fn visit(&mut self, path: PathBuf, file_type: FileType, components: &'p [GlobComponent]) {
        let Some((first, rest)) = components.split_first() else {
            return;
        };
        if matches!(first, GlobComponent::AnyDirs) {
            // `**` matches zero directories, or this one and any below it
            let name = path.file_name().unwrap_or_default();
            if rest.first().is_some_and(|next| next.matches(name)) {
                self.visit(path.clone(), file_type, rest);
            }
            if file_type.is_dir() {
                self.todo.push((path, components));
            }
        } else if file_type.is_symlink() {
            debug!("glob: skipping symlink {}", path.display());
        } else if rest.is_empty() && file_type.is_file() {
            self.matches.push(path);
        } else if !rest.is_empty() && file_type.is_dir() {
            self.todo.push((path, rest));
        }
    }
}

/// The entries of `dir` that `component` matches, with their own types: a
/// symlink is reported as one, not as its target. A literal is looked up
/// directly, so it needs only traverse permission on `dir`. Reparse points are
/// left out, since `open_regular_file` refuses them.
fn glob_entries(dir: &Path, component: &GlobComponent) -> Vec<(PathBuf, FileType)> {
    if let GlobComponent::Literal(name) = component {
        // `..` would lead the walk back out of the prefix
        if name == ".." {
            return Vec::new();
        }
        let path = dir.join(name);
        return match std::fs::symlink_metadata(&path) {
            Ok(meta) if !is_reparse_point(&meta) => vec![(path, meta.file_type())],
            _ => Vec::new(),
        };
    }
    // an empty `dir` is the current directory
    let read_from = match dir.as_os_str().is_empty() {
        true => Path::new("."),
        false => dir,
    };
    let Ok(entries) = std::fs::read_dir(read_from) else {
        return Vec::new();
    };
    let entries = entries
        .flatten()
        .filter(|entry| component.matches(&entry.file_name()));
    // on Windows the listing already holds each entry's attributes, so this
    // needs no extra system call
    #[cfg(windows)]
    let entries = entries.filter(|e| !e.metadata().is_ok_and(|m| is_reparse_point(&m)));
    entries
        .filter_map(|entry| Some((dir.join(entry.file_name()), entry.file_type().ok()?)))
        .collect()
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
