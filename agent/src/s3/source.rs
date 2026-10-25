// internal crates
use crate::filesys::{errors::PathDoesNotExistErr, file::File, path::PathExt, FileSysErr};
use crate::s3::{errors, gcs, Object, S3Err};
use crate::trace;

// external crates
use aws_sdk_s3::primitives::{ByteStream, Length};
use base64::Engine;

/// Bytes per read and per frame sent. The SDK's default is 4 KiB.
const READ_SIZE: usize = 32 * 1024;

/// A local file to upload, its size when the upload started, and the CRC32C
/// the uploaded bytes must match. The file is expected to stay unchanged until
/// the upload is confirmed.
pub struct Source {
    file: File,
    pub(super) size: u64,
    pub(super) crc32c: Option<u32>,
}

impl Source {
    /// Sizes `file` for upload, checked against `crc32c` when there is one. A
    /// missing file is [`S3Err::FileSysErr`]; any other metadata failure is
    /// [`S3Err::LocalIoErr`].
    pub async fn open(file: File, crc32c: Option<u32>, dst: &Object) -> Result<Self, S3Err> {
        let metadata = tokio::fs::metadata(file.path()).await.map_err(|e| {
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
        Ok(Self {
            file,
            size: metadata.len(),
            crc32c,
        })
    }

    /// The CRC32C as S3 and GCS headers carry it: the base64 of its big-endian
    /// bytes.
    pub(super) fn crc32c_base64(&self) -> Option<String> {
        let crc32c = self.crc32c?;
        Some(base64::engine::general_purpose::STANDARD.encode(crc32c.to_be_bytes()))
    }

    /// A request body that streams `len` bytes from `offset`. A retry reopens
    /// the file. A file now shorter than the range is [`S3Err::LocalIoErr`]
    /// before anything is read.
    pub(super) async fn read(
        &self,
        operation: &str,
        dst: &Object,
        offset: u64,
        len: u64,
    ) -> Result<ByteStream, S3Err> {
        ByteStream::read_from()
            .path(self.file.path())
            .offset(offset)
            .length(Length::Exact(len))
            .buffer_size(READ_SIZE)
            .build()
            .await
            .map_err(|e| errors::map_body_io_err(operation, dst, &self.file, e.into()))
    }

    /// The `Content-MD5` of `len` bytes from `offset` (see
    /// [`gcs::content_md5`]), hashed in its own read of the range.
    pub(super) async fn content_md5(
        &self,
        operation: &str,
        dst: &Object,
        offset: u64,
        len: u64,
    ) -> Result<String, S3Err> {
        let range = self.read(operation, dst, offset, len).await?;
        gcs::content_md5(range)
            .await
            .map_err(|e| errors::map_body_io_err(operation, dst, &self.file, e))
    }
}
