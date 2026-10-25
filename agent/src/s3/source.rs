// standard crates
use std::io::SeekFrom;

// internal crates
use crate::filesys::{errors::PathDoesNotExistErr, file::File, path::PathExt, FileSysErr};
use crate::s3::{errors, Object, S3Err};
use crate::trace;

// external crates
use tokio::io::{AsyncReadExt, AsyncSeekExt};

/// A local file opened once for upload. Its size and every byte read come from
/// the same open handle, so the upload stays bound to the opened file even if
/// its path is replaced afterwards.
pub struct Source {
    file: File,
    handle: tokio::fs::File,
    pub(super) size: u64,
}

impl Source {
    /// Opens `file` for reading. A missing file is [`S3Err::FileSysErr`]; any
    /// other open or metadata failure is [`S3Err::LocalIoErr`].
    pub async fn open(file: File, dst: &Object) -> Result<Self, S3Err> {
        let handle = tokio::fs::File::open(file.path()).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                FileSysErr::PathDoesNotExistErr(PathDoesNotExistErr {
                    path: file.path().clone(),
                    trace: trace!(),
                })
                .into()
            } else {
                errors::map_body_io_err("open_source", dst, &file, e)
            }
        })?;
        let size = handle
            .metadata()
            .await
            .map_err(|e| errors::map_body_io_err("open_source", dst, &file, e))?
            .len();
        Ok(Self { file, handle, size })
    }

    /// Reads `[offset..offset+length]` into memory, mapping any seek/read failure
    /// to [`S3Err::LocalIoErr`]. A file truncated below `offset + length` after
    /// it was opened makes `read_exact` fail with `UnexpectedEof`, which maps to
    /// the same error.
    pub(super) async fn read(
        &mut self,
        operation: &str,
        dst: &Object,
        offset: u64,
        length: u64,
    ) -> Result<Vec<u8>, S3Err> {
        self.handle
            .seek(SeekFrom::Start(offset))
            .await
            .map_err(|e| errors::map_body_io_err(operation, dst, &self.file, e))?;
        let mut buf = vec![0u8; length as usize];
        self.handle
            .read_exact(&mut buf)
            .await
            .map_err(|e| errors::map_body_io_err(operation, dst, &self.file, e))?;
        Ok(buf)
    }
}
