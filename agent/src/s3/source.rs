// standard crates
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{ready, Context, Poll};

// internal crates
use crate::filesys::{errors::PathDoesNotExistErr, file::File, files, path::PathExt, FileSysErr};
use crate::s3::{errors, gcs, Object, S3Err};
use crate::trace;

// external crates
use aws_sdk_s3::primitives::ByteStream;
use aws_smithy_types::body::SdkBody;
use bytes::Bytes;
use http_body::{Body, Frame, SizeHint};
use tokio::task::JoinHandle;

/// Bytes per read and per frame sent. The SDK's default is 4 KiB.
const READ_SIZE: usize = 32 * 1024;

/// A local file to upload and its size when the upload started. The file is
/// expected to stay unchanged until the upload is confirmed.
pub struct Source {
    file: File,
    /// Held open so every read gets the file checked in [`Source::open`], even
    /// if its path is swapped later.
    handle: Arc<std::fs::File>,
    pub(crate) size: u64,
}

impl Source {
    /// Opens and sizes `file` for upload with [`files::open_regular_file`],
    /// which refuses a symlink. A missing file is
    /// [`S3Err::FileSysErr`]; a symlink, anything else that isn't a regular
    /// file, or any other failure is [`S3Err::LocalIoErr`].
    pub async fn open(file: File, dst: &Object) -> Result<Self, S3Err> {
        let map_err = |e: io::Error| -> S3Err {
            if e.kind() == io::ErrorKind::NotFound {
                FileSysErr::PathDoesNotExistErr(PathDoesNotExistErr {
                    path: file.path().clone(),
                    trace: trace!(),
                })
                .into()
            } else {
                errors::map_body_io_err("open_source", dst, &file, e)
            }
        };
        let handle = files::open_regular_file(&file).await.map_err(map_err)?;
        let size = handle.metadata().await.map_err(map_err)?.len();
        Ok(Self {
            file,
            handle: Arc::new(handle.into_std().await),
            size,
        })
    }

    /// A request body that streams `len` bytes from `offset` of the open file.
    /// A retry reads the range again. A file now shorter than the range is
    /// [`S3Err::LocalIoErr`] before anything is read.
    pub(super) async fn read(
        &self,
        operation: &str,
        dst: &Object,
        offset: u64,
        len: u64,
    ) -> Result<ByteStream, S3Err> {
        let map_err = |e| errors::map_body_io_err(operation, dst, &self.file, e);
        // fstat can block on a slow filesystem, so keep it off the runtime
        let file = self.handle.clone();
        let stat = tokio::task::spawn_blocking(move || file.metadata());
        let size = match stat.await {
            Ok(metadata) => metadata.map_err(map_err)?.len(),
            Err(e) => return Err(map_err(e.into())),
        };
        if size < offset + len {
            let kind = io::ErrorKind::UnexpectedEof;
            let e = io::Error::new(kind, "file is shorter than the upload range");
            return Err(map_err(e));
        }
        let handle = self.handle.clone();
        let body = SdkBody::retryable(move || {
            SdkBody::from_body_1_x(RangeBody {
                file: handle.clone(),
                pos: offset,
                end: offset + len,
                reading: None,
            })
        });
        Ok(ByteStream::new(body))
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

/// The bytes from `pos` to `end` of a file. Reads are positional, so bodies
/// sharing one handle never move each other's place in the file.
struct RangeBody {
    file: Arc<std::fs::File>,
    pos: u64,
    end: u64,
    reading: Option<JoinHandle<io::Result<Bytes>>>,
}

impl Body for RangeBody {
    type Data = Bytes;
    type Error = io::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<io::Result<Frame<Bytes>>>> {
        let this = &mut *self;
        if this.pos >= this.end {
            return Poll::Ready(None);
        }
        let reading = this.reading.get_or_insert_with(|| {
            let (file, pos) = (this.file.clone(), this.pos);
            let len = (this.end - pos).min(READ_SIZE as u64) as usize;
            tokio::task::spawn_blocking(move || read_at(&file, pos, len))
        });
        let read = ready!(Pin::new(reading).poll(cx));
        this.reading = None;
        let chunk = read??;
        if chunk.is_empty() {
            return Poll::Ready(Some(Err(io::ErrorKind::UnexpectedEof.into())));
        }
        this.pos += chunk.len() as u64;
        Poll::Ready(Some(Ok(Frame::data(chunk))))
    }

    fn is_end_stream(&self) -> bool {
        self.pos >= self.end
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::with_exact(self.end - self.pos)
    }
}

/// Reads up to `len` bytes at `pos`.
fn read_at(file: &std::fs::File, pos: u64, len: usize) -> io::Result<Bytes> {
    let mut buf = vec![0; len];
    #[cfg(unix)]
    let n = std::os::unix::fs::FileExt::read_at(file, &mut buf, pos)?;
    #[cfg(windows)]
    let n = std::os::windows::fs::FileExt::seek_read(file, &mut buf, pos)?;
    buf.truncate(n);
    Ok(buf.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::s3::store::tests::{obj, temp_file_with};
    use crate::test_utils::filesys::missing_file;

    #[tokio::test]
    async fn open_sizes_regular_file() {
        let tf = temp_file_with(b"hello").await;
        let src = Source::open(tf.to_file(), &obj("k")).await.unwrap();
        assert_eq!(src.size, 5);
    }

    #[tokio::test]
    async fn open_missing_file_maps_to_filesys_err() {
        let Err(err) = Source::open(missing_file(), &obj("k")).await else {
            panic!("opened a missing file");
        };
        assert!(matches!(
            err,
            S3Err::FileSysErr(FileSysErr::PathDoesNotExistErr(_))
        ));
    }

    #[tokio::test]
    async fn open_unopenable_file_maps_to_local_io_err() {
        // Every platform rejects a path with an interior NUL byte as
        // `InvalidInput` rather than `NotFound`, so the open fails as
        // `LocalIoErr`.
        let src = File::new("/tmp/bad\0name.bin");

        let Err(err) = Source::open(src, &obj("k")).await else {
            panic!("opened a path with a NUL byte");
        };

        let S3Err::LocalIoErr(inner) = &err else {
            panic!("expected LocalIoErr, got {err:?}");
        };
        assert_eq!(
            (inner.operation.as_str(), &inner.object),
            ("open_source", &obj("k"))
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn open_symlink_maps_to_local_io_err() {
        // The file is opened without following a symlink.
        let target = temp_file_with(b"secret").await;
        let dir = crate::test_utils::filesys::dirs::temp("s3-test").unwrap();
        let link = dir.file("x.log");
        std::os::unix::fs::symlink(target.file().path(), link.path()).unwrap();

        let Err(err) = Source::open(link, &obj("k")).await else {
            panic!("opened a symlink");
        };

        let S3Err::LocalIoErr(inner) = &err else {
            panic!("expected LocalIoErr, got {err:?}");
        };
        assert_eq!(
            (inner.operation.as_str(), &inner.object),
            ("open_source", &obj("k"))
        );
    }

    #[tokio::test]
    async fn read_streams_range() {
        let tf = temp_file_with(b"hello world").await;
        let src = Source::open(tf.to_file(), &obj("k")).await.unwrap();

        let body = src.read("put_object", &obj("k"), 6, 5).await.unwrap();

        assert_eq!(body.size_hint(), (5, Some(5)));
        assert_eq!(body.collect().await.unwrap().to_vec(), b"world");
    }

    #[tokio::test]
    async fn read_streams_opened_file_after_path_swap() {
        let dir = crate::test_utils::filesys::dirs::temp("s3-test").unwrap();
        let file = dir.file("x.log");
        std::fs::write(file.path(), b"original").unwrap();
        let src = Source::open(file.clone(), &obj("k")).await.unwrap();
        std::fs::rename(file.path(), dir.file("moved.log").path()).unwrap();
        // shorter, so a size check by path would fail
        std::fs::write(file.path(), b"abc").unwrap();

        let body = src.read("put_object", &obj("k"), 0, 8).await.unwrap();

        assert_eq!(body.collect().await.unwrap().to_vec(), b"original");
    }

    #[tokio::test]
    async fn read_fails_when_file_shrinks_mid_body() {
        // The file covered the range when the body was built, so it's the
        // read itself that runs out of file.
        let tf = temp_file_with(b"hello world").await;
        let src = Source::open(tf.to_file(), &obj("k")).await.unwrap();
        let body = src.read("put_object", &obj("k"), 0, 11).await.unwrap();
        std::fs::File::options()
            .write(true)
            .open(tf.to_file().path())
            .unwrap()
            .set_len(4)
            .unwrap();

        let Err(err) = body.collect().await else {
            panic!("read past the end of the file");
        };
        let err: &(dyn std::error::Error + 'static) = &err;
        let io_err = std::iter::successors(Some(err), |e| e.source())
            .find_map(|e| e.downcast_ref::<io::Error>())
            .expect("an io::Error in the source chain");
        assert_eq!(io_err.kind(), io::ErrorKind::UnexpectedEof);
    }
}
