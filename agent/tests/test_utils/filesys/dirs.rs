// internal crates
use crate::filesys::{
    errors::{CreateTmpDirErr, FileSysErr},
    Dir,
};
use crate::trace;

/// RAII temp directory for TESTS. Owns a `tempfile::TempDir` (Drop deletes the
/// dir) plus our `Dir` handle; the directory lives exactly as long as this value.
#[derive(Debug)]
pub(crate) struct TempDir {
    _guard: tempfile::TempDir,
    dir: Dir,
}

impl TempDir {
    pub(crate) fn dir(&self) -> &Dir {
        &self.dir
    }

    /// Owned `Dir` to move into a longer-lived owner; valid only while `self` lives.
    pub(crate) fn to_dir(&self) -> Dir {
        self.dir.clone()
    }
}

impl std::ops::Deref for TempDir {
    type Target = Dir;
    fn deref(&self) -> &Dir {
        &self.dir
    }
}

/// Auto-cleaning temp dir for tests. Sync; keeps `prefix` for parity with
/// [`crate::filesys::dirs::create_temp`]. The returned [`TempDir`] deletes the
/// directory on drop, so bind it to a named variable that lives as long as needed.
pub(crate) fn temp(prefix: &str) -> Result<TempDir, FileSysErr> {
    let guard = tempfile::Builder::new()
        .prefix(prefix)
        .tempdir()
        .map_err(|e| {
            FileSysErr::CreateTmpDirErr(CreateTmpDirErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })?;
    let dir = Dir::new(guard.path().to_path_buf());
    Ok(TempDir { _guard: guard, dir })
}
