// internal crates
use crate::filesys::{errors::PathDoesNotExistErr, file::File, path::PathExt, FileSysErr};
use crate::s3::{errors, gcs, Object, S3Err};
use crate::trace;

// external crates
use aws_sdk_s3::primitives::{ByteStream, Length};

/// Bytes per read and per frame sent. The SDK's default is 4 KiB.
const READ_SIZE: usize = 32 * 1024;

/// A local file to upload and its size when the upload started. The file is
/// expected to stay unchanged until the upload is confirmed.
pub struct Source {
    file: File,
    pub(crate) size: u64,
}

impl Source {
    /// Sizes `file` for upload. A missing file is [`S3Err::FileSysErr`]; a
    /// symlink or any other metadata failure is [`S3Err::LocalIoErr`].
    pub async fn open(file: File, dst: &Object) -> Result<Self, S3Err> {
        let metadata = tokio::fs::symlink_metadata(file.path())
            .await
            .map_err(|e| {
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
        if metadata.file_type().is_symlink() {
            let e = std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "refusing to upload a symlink",
            );
            return Err(errors::map_body_io_err("open_source", dst, &file, e));
        }
        Ok(Self {
            file,
            size: metadata.len(),
        })
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::s3::store::tests::{obj, temp_file_with};

    #[tokio::test]
    async fn open_refuses_symlink() {
        let target = temp_file_with(b"secret").await;
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("x.log");
        std::os::unix::fs::symlink(target.to_file().path(), &link).unwrap();

        let err = Source::open(File::new(link), &obj("k"))
            .await
            .err()
            .unwrap();
        assert!(matches!(err, S3Err::LocalIoErr(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn open_sizes_regular_file() {
        let tf = temp_file_with(b"hello").await;
        let src = Source::open(tf.to_file(), &obj("k")).await.unwrap();
        assert_eq!(src.size, 5);
    }
}
