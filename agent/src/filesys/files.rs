// standard crates
use std::ffi::OsStr;
use std::io::Write;
#[cfg(windows)]
use std::os::windows::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime};

// internal crates
use crate::cooldown;
use crate::filesys::{
    dir::Dir,
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
/// The file is opened beneath `root` with [`open_regular_file`].
///
/// Streams the file through a fixed 8 KiB buffer, so memory use is constant
/// regardless of file size (the whole file is never materialized). Hashing is
/// CPU-heavy, so callers may wrap this in `tokio::task::spawn_blocking` for very
/// large files to avoid stalling the async runtime.
pub async fn sha256(root: &Dir, file: &File) -> Result<String, FileSysErr> {
    let mut hasher = Sha256::new();
    read_chunks(root, file, |chunk| hasher.update(chunk)).await?;
    let digest = hasher.finalize();
    Ok(format!("sha256:{digest:x}"))
}

/// Computes `file`'s [`sha256`] digest and its CRC32C (Castagnoli) checksum in
/// one read.
pub async fn sha256_and_crc32c(root: &Dir, file: &File) -> Result<(String, u32), FileSysErr> {
    let mut hasher = Sha256::new();
    let mut crc32c = crc_fast::Digest::new(CrcAlgorithm::Crc32Iscsi);
    read_chunks(root, file, |chunk| {
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
async fn read_chunks(
    root: &Dir,
    file: &File,
    mut on_chunk: impl FnMut(&[u8]),
) -> Result<(), FileSysErr> {
    let mut f = open_regular_file(root, file)
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
/// a symlink at `file` or, on Unix, at any directory between `root` and `file`.
/// So a link planted in a watched directory, or swapped in for one of its
/// directories, is never read through. `root` itself may be a symlink. On
/// Windows it also refuses every other reparse point.
pub async fn open_regular_file(root: &Dir, file: &File) -> std::io::Result<TokioFile> {
    let f = open_beneath(root, file).await?;
    let meta = f.metadata().await?;
    // on Windows the no-follow flag also skips a placeholder's filter, so its
    // stub would be read
    if !meta.is_file() || is_reparse_point(&meta) {
        let kind = std::io::ErrorKind::InvalidInput;
        return Err(std::io::Error::new(kind, "not a regular file"));
    }
    Ok(f)
}

#[cfg(unix)]
async fn open_beneath(root: &Dir, file: &File) -> std::io::Result<TokioFile> {
    use nix::fcntl::{openat, OFlag};
    use nix::sys::stat::Mode;
    use std::os::fd::OwnedFd;

    let (root, file) = (root.clone(), file.clone());
    let fd = tokio::task::spawn_blocking(move || -> std::io::Result<OwnedFd> {
        let (dir, name) = open_parent_beneath(&root, &file)?;
        // O_NONBLOCK keeps a FIFO from blocking the open until it's refused
        let flags = OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC;
        Ok(openat(dir, name, flags, Mode::empty())?)
    })
    .await??;
    Ok(TokioFile::from_std(std::fs::File::from(fd)))
}

#[cfg(windows)]
async fn open_beneath(_root: &Dir, file: &File) -> std::io::Result<TokioFile> {
    let mut opts = tokio::fs::OpenOptions::new();
    opts.read(true);
    // opens a link itself rather than its target, so the caller refuses it
    opts.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    opts.open(file.path()).await
}

/// Opens the directory holding `file` and returns it with `file`'s name. It
/// opens `root`, following links, then each directory below it without
/// following one, so a symlink below `root` fails the walk. A `file` that
/// isn't a plain path below `root` is refused.
#[cfg(unix)]
fn open_parent_beneath<'f>(
    root: &Dir,
    file: &'f File,
) -> std::io::Result<(std::os::fd::OwnedFd, &'f OsStr)> {
    use nix::fcntl::{openat, OFlag, AT_FDCWD};
    use nix::sys::stat::Mode;

    let outside = || std::io::Error::new(std::io::ErrorKind::InvalidInput, "not below its root");
    let below = file
        .path()
        .strip_prefix(root.path())
        .map_err(|_| outside())?;
    let names = below
        .components()
        .map(|c| match c {
            Component::Normal(name) => Ok(name),
            _ => Err(outside()),
        })
        .collect::<std::io::Result<Vec<_>>>()?;
    let Some((name, parents)) = names.split_last() else {
        return Err(outside());
    };
    // an empty root is the current directory, as in `glob`
    let root = match root.path().as_os_str().is_empty() {
        true => Path::new("."),
        false => root.path().as_path(),
    };
    let flags = OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC;
    let mut dir = openat(AT_FDCWD, root, flags, Mode::empty())?;
    for parent in parents {
        dir = openat(&dir, *parent, flags | OFlag::O_NOFOLLOW, Mode::empty())?;
    }
    Ok((dir, *name))
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

/// Deletes `file` like [`delete`]. On Unix it reaches `file` as
/// [`open_regular_file`] does, so a symlink swapped in for a directory between
/// `root` and `file` fails the delete instead of being followed. Windows
/// follows links.
#[cfg(unix)]
pub async fn delete_beneath(root: &Dir, file: &File) -> Result<(), FileSysErr> {
    use nix::unistd::{unlinkat, UnlinkatFlags};

    let (owned_root, owned_file) = (root.clone(), file.clone());
    let unlinked = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        let (dir, name) = open_parent_beneath(&owned_root, &owned_file)?;
        Ok(unlinkat(dir, name, UnlinkatFlags::NoRemoveDir)?)
    })
    .await;
    match unlinked.unwrap_or_else(|e| Err(e.into())) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(FileSysErr::DeleteFileErr(DeleteFileErr {
            source: Box::new(e),
            file: file.clone(),
            trace: trace!(),
        })),
    }
}

#[cfg(windows)]
pub async fn delete_beneath(_root: &Dir, file: &File) -> Result<(), FileSysErr> {
    delete(file).await
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
    if names_only_dirs(pattern) {
        return Ok(Vec::new());
    }
    let files = parsed
        .walk()
        .filter_map(|entry| parsed.matching_file(&entry))
        .collect();
    Ok(files)
}

/// The directory [`glob`] walks `pattern` from: its literal prefix, the
/// directories before the first wildcard. No file [`glob`] returns has a
/// symlink between this directory and itself.
pub fn glob_root(pattern: &str) -> Dir {
    let components: Vec<_> = Path::new(pattern).components().collect();
    let literal = literal_prefix_len(&components);
    Dir::new(components[..literal].iter().collect::<PathBuf>())
}

/// Checks that `pattern` is a glob [`glob`] accepts, without walking the
/// filesystem. An invalid pattern returns [`FileSysErr::InvalidGlobErr`].
pub fn validate_glob(pattern: &str) -> Result<(), FileSysErr> {
    ParsedGlob::new(pattern).map(drop)
}

/// `*` and `?` never match a separator, as with `glob::glob`.
const GLOB_MATCH_OPTIONS: glob::MatchOptions = glob::MatchOptions {
    case_sensitive: true,
    require_literal_separator: true,
    require_literal_leading_dot: false,
};

/// A glob pattern split at its literal prefix, where the walk starts.
struct ParsedGlob {
    root: PathBuf,
    /// The pattern below `root`, matched against each file's path below it.
    below: glob::Pattern,
    /// The pattern a directory must match at each level below `root`, up to
    /// the first `**`.
    dir_patterns: Vec<glob::Pattern>,
    /// How deep below `root` a match can be; any depth if there's a `**`.
    max_depth: Option<usize>,
}

impl ParsedGlob {
    fn new(pattern: &str) -> Result<Self, FileSysErr> {
        let components: Vec<_> = Path::new(pattern).components().collect();
        let (root, below) = components.split_at(literal_prefix_len(&components));
        let dirs = below.split_last().map_or(&[][..], |(_, dirs)| dirs);
        let below_path: PathBuf = below.iter().collect();
        Ok(Self {
            root: root.iter().collect(),
            below: compile_glob(pattern, &below_path.to_string_lossy())?,
            dir_patterns: dir_patterns(pattern, dirs)?,
            max_depth: (!has_recursive_wildcard(below)).then_some(below.len()),
        })
    }

    /// Where the walk starts: `root`, or the current directory if it's empty.
    fn walk_root(&self) -> &Path {
        if self.root.as_os_str().is_empty() {
            Path::new(".")
        } else {
            &self.root
        }
    }

    /// The entries below `root` that can hold a match, without following
    /// symlinks.
    fn walk(&self) -> impl Iterator<Item = walkdir::DirEntry> + '_ {
        let mut walk = WalkDir::new(self.walk_root())
            .min_depth(1)
            .sort_by_file_name();
        if let Some(max_depth) = self.max_depth {
            walk = walk.max_depth(max_depth);
        }
        walk.into_iter()
            .filter_entry(|entry| !is_skipped_reparse_point(entry) && self.may_hold_match(entry))
            .flatten()
    }

    /// Whether the walk should keep `entry`: a directory is entered only if
    /// its name matches the pattern for its level.
    fn may_hold_match(&self, entry: &walkdir::DirEntry) -> bool {
        if !entry.file_type().is_dir() {
            return true;
        }
        // the walk's root is level 0
        let Some(level) = entry.depth().checked_sub(1) else {
            return true;
        };
        let name = entry.file_name().to_string_lossy();
        // past the first `**`, any directory can hold a match
        self.dir_patterns
            .get(level)
            .is_none_or(|dir_pattern| dir_pattern.matches(&name))
    }

    /// `entry` as a [`File`], if it's a regular file that matches the pattern.
    fn matching_file(&self, entry: &walkdir::DirEntry) -> Option<File> {
        if entry.path_is_symlink() {
            debug!("glob: skipping symlink {}", entry.path().display());
            return None;
        }
        if !entry.file_type().is_file() {
            return None;
        }
        let relative = entry.path().strip_prefix(self.walk_root()).ok()?;
        let is_match = self.below.matches_path_with(relative, GLOB_MATCH_OPTIONS);
        is_match.then(|| File::new(self.root.join(relative)))
    }
}

/// How many leading `components` form a glob's literal prefix: those with no
/// wildcard, never the last, since it names the files to match.
fn literal_prefix_len(components: &[Component]) -> usize {
    let dirs = &components[..components.len().saturating_sub(1)];
    dirs.iter()
        .take_while(|c| !has_wildcard(c.as_os_str()))
        .count()
}

fn has_wildcard(name: &OsStr) -> bool {
    name.to_string_lossy().contains(['*', '?', '['])
}

fn has_recursive_wildcard(components: &[Component]) -> bool {
    components.iter().any(|c| c.as_os_str() == "**")
}

/// The pattern for each directory level in `dirs`, up to the first `**`.
fn dir_patterns(pattern: &str, dirs: &[Component]) -> Result<Vec<glob::Pattern>, FileSysErr> {
    dirs.iter()
        .map(|c| c.as_os_str().to_string_lossy())
        .take_while(|name| *name != "**")
        .map(|name| compile_glob(pattern, &name))
        .collect()
}

/// Compiles `part` of the glob `pattern`, naming the whole pattern on error.
fn compile_glob(pattern: &str, part: &str) -> Result<glob::Pattern, FileSysErr> {
    glob::Pattern::new(part).map_err(|e| {
        FileSysErr::InvalidGlobErr(InvalidGlobErr {
            pattern: pattern.to_string(),
            source: Box::new(e),
            trace: trace!(),
        })
    })
}

/// Whether `pattern` names only directories, as with `glob::glob`: it ends
/// in `**`, `.` or a separator. It reads the raw text, since
/// [`Path::components`] drops a trailing `.` or separator.
fn names_only_dirs(pattern: &str) -> bool {
    let last = pattern.rsplit(std::path::is_separator).next();
    matches!(last, Some("" | "." | "**"))
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
