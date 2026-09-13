// internal crates
use miru_agent::filesys::{
    errors::{CreateTmpFileErr, FileSysErr},
    files::write_string,
    File, WriteOptions,
};
use miru_agent::trace;

/// RAII temp file for TESTS. Owns a `tempfile::NamedTempFile` (Drop deletes the
/// file) plus our `File` handle; the file lives exactly as long as this value.
#[derive(Debug)]
pub struct TempFile {
    _guard: tempfile::NamedTempFile,
    file: File,
}

impl TempFile {
    pub(crate) fn file(&self) -> &File {
        &self.file
    }

    /// Owned `File` to move into a longer-lived owner; valid only while `self` lives.
    pub fn to_file(&self) -> File {
        self.file.clone()
    }
}

impl std::ops::Deref for TempFile {
    type Target = File;
    fn deref(&self) -> &File {
        &self.file
    }
}

/// Auto-cleaning temp file for tests. Sync; keeps `prefix` for parity with
/// [`super::dirs::temp`]. The returned [`TempFile`] deletes the file on
/// drop, so bind it to a named variable that lives as long as the file is needed.
pub(crate) fn temp(prefix: &str) -> Result<TempFile, FileSysErr> {
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

/// Test-only convenience: atomically (over)write `contents` to `file`, panicking
/// on error. Collapses the common `write_string(f, s, OVERWRITE_ATOMIC).await
/// .unwrap()` seed pattern to a single line. For non-default write options, call
/// [`write_string`] directly.
pub(crate) async fn seed(file: &File, contents: &str) {
    write_string(file, contents, WriteOptions::OVERWRITE_ATOMIC)
        .await
        .unwrap();
}
