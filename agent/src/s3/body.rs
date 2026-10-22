// standard crates
use std::convert::Infallible;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{ready, Context, Poll};

// external crates
use aws_sdk_s3::primitives::ByteStream;
use aws_smithy_types::body::SdkBody;
use bytes::Bytes;
use http_body::{Body, Frame, SizeHint};
use tokio::task::JoinHandle;

pub(crate) const FRAME_SIZE: usize = 32 * 1024;

/// Request body for PutObject/UploadPart. Larger bodies are yielded in
/// [`FRAME_SIZE`] frames so stalled-stream protection sees per-frame progress;
/// retryable because each retry rebuilds from a cheap `Bytes` clone.
pub(crate) fn chunked(bytes: Bytes) -> ByteStream {
    if bytes.len() <= FRAME_SIZE {
        return ByteStream::from(bytes);
    }
    ByteStream::new(SdkBody::retryable(move || {
        SdkBody::from_body_1_x(ChunkedBytes {
            remaining: bytes.clone(),
        })
    }))
}

struct ChunkedBytes {
    remaining: Bytes,
}

impl Body for ChunkedBytes {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let remaining = &mut self.get_mut().remaining;
        if remaining.is_empty() {
            return Poll::Ready(None);
        }
        let frame = remaining.split_to(FRAME_SIZE.min(remaining.len()));
        Poll::Ready(Some(Ok(Frame::data(frame))))
    }

    fn is_end_stream(&self) -> bool {
        self.remaining.is_empty()
    }

    // Exact, so the SDK can send Content-Length and the aws-chunked checksum
    // trailer.
    fn size_hint(&self) -> SizeHint {
        SizeHint::with_exact(self.remaining.len() as u64)
    }
}

/// Request body that streams `[offset, offset + len)` of an opened file in
/// [`FRAME_SIZE`] frames with positional reads, so at most one frame of the
/// range is in memory. Every byte comes from `file` itself, never its path.
/// Retryable: each retry restarts the range from `offset`. A failed or short
/// read ends the body with an error and is kept in `failure`.
pub(crate) fn file_range(
    file: Arc<std::fs::File>,
    offset: u64,
    len: u64,
    failure: ReadFailure,
) -> ByteStream {
    ByteStream::new(SdkBody::retryable(move || {
        SdkBody::from_body_1_x(FileRange {
            file: file.clone(),
            offset,
            remaining: len,
            pending: None,
            failure: failure.clone(),
        })
    }))
}

/// The last local read error of a [`file_range`] body. The SDK reports a body
/// error as a dispatch failure, so callers check this to tell a local read
/// failure from a network one.
#[derive(Clone, Default)]
pub(crate) struct ReadFailure(Arc<Mutex<Option<io::Error>>>);

impl ReadFailure {
    fn record(&self, err: &io::Error) {
        let copy = io::Error::new(err.kind(), err.to_string());
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(copy);
    }

    pub(crate) fn take(&self) -> Option<io::Error> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}

struct FileRange {
    file: Arc<std::fs::File>,
    offset: u64,
    remaining: u64,
    pending: Option<JoinHandle<io::Result<Vec<u8>>>>,
    failure: ReadFailure,
}

impl FileRange {
    fn fail(&mut self, err: io::Error) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
        self.failure.record(&err);
        self.remaining = 0;
        Poll::Ready(Some(Err(err)))
    }
}

impl Body for FileRange {
    type Data = Bytes;
    type Error = io::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        if this.remaining == 0 {
            return Poll::Ready(None);
        }
        let pending = this.pending.get_or_insert_with(|| {
            let file = this.file.clone();
            let offset = this.offset;
            let len = this.remaining.min(FRAME_SIZE as u64) as usize;
            tokio::task::spawn_blocking(move || {
                let mut buf = vec![0u8; len];
                read_exact_at(&file, &mut buf, offset)?;
                Ok(buf)
            })
        });
        let joined = ready!(Pin::new(pending).poll(cx));
        this.pending = None;
        match joined.unwrap_or_else(|e| Err(io::Error::other(e))) {
            Ok(buf) => {
                this.offset += buf.len() as u64;
                this.remaining -= buf.len() as u64;
                Poll::Ready(Some(Ok(Frame::data(Bytes::from(buf)))))
            }
            Err(err) => this.fail(err),
        }
    }

    fn is_end_stream(&self) -> bool {
        self.remaining == 0
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::with_exact(self.remaining)
    }
}

/// Fills `buf` from `file` at `offset` without moving a shared cursor. A file
/// that ends before `buf` is full is [`io::ErrorKind::UnexpectedEof`].
pub(crate) fn read_exact_at(
    file: &std::fs::File,
    mut buf: &mut [u8],
    mut offset: u64,
) -> io::Result<()> {
    while !buf.is_empty() {
        match read_at(file, buf, offset) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "file ended before the requested range",
                ))
            }
            Ok(n) => {
                buf = &mut buf[n..];
                offset += n as u64;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(unix)]
fn read_at(file: &std::fs::File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    std::os::unix::fs::FileExt::read_at(file, buf, offset)
}

#[cfg(windows)]
fn read_at(file: &std::fs::File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    std::os::windows::fs::FileExt::seek_read(file, buf, offset)
}

#[cfg(test)]
mod tests {
    // internal crates
    use super::*;
    use crate::filesys::path::PathExt;
    use crate::s3::tests::temp_file_with;

    // external crates
    use http_body_util::BodyExt;

    const KIB: usize = 1024;

    /// Bytes with a period (251) that no frame size divides, so a dropped or
    /// reordered frame cannot go unnoticed.
    fn pattern(len: usize) -> Bytes {
        Bytes::from((0..len).map(|i| (i % 251) as u8).collect::<Vec<u8>>())
    }

    /// Polls `body` to its end, returning each data frame.
    async fn drain(body: &mut SdkBody) -> Vec<Bytes> {
        let mut frames = Vec::new();
        while let Some(frame) = body.frame().await {
            frames.push(frame.unwrap().into_data().unwrap());
        }
        frames
    }

    fn lens(frames: &[Bytes]) -> Vec<usize> {
        frames.iter().map(Bytes::len).collect()
    }

    #[test]
    fn small_body_stays_in_memory() {
        let input = pattern(FRAME_SIZE);

        let body = chunked(input.clone()).into_inner();

        assert_eq!(body.bytes(), Some(&input[..]));
        assert!(!body.is_streaming());
    }

    #[tokio::test]
    async fn large_body_yields_bounded_frames() {
        let input = pattern(100 * KIB);
        let mut body = chunked(input.clone()).into_inner();
        assert!(body.bytes().is_none());

        let frames = drain(&mut body).await;

        assert_eq!(lens(&frames), vec![32 * KIB, 32 * KIB, 32 * KIB, 4 * KIB]);
        assert_eq!(frames.concat(), input.to_vec());
    }

    #[tokio::test]
    async fn one_byte_over_frame_size_streams() {
        let input = pattern(FRAME_SIZE + 1);
        let mut body = chunked(input.clone()).into_inner();
        assert!(body.bytes().is_none());

        let frames = drain(&mut body).await;

        assert_eq!(lens(&frames), vec![FRAME_SIZE, 1]);
        assert_eq!(frames.concat(), input.to_vec());
    }

    #[tokio::test]
    async fn size_hint_is_exact_and_end_stream_tracks_remaining() {
        let mut body = ChunkedBytes {
            remaining: pattern(40 * KIB),
        };
        assert_eq!(body.size_hint().exact(), Some(40 * KIB as u64));
        assert!(!body.is_end_stream());

        body.frame().await.unwrap().unwrap();
        assert_eq!(body.size_hint().exact(), Some(8 * KIB as u64));
        assert!(!body.is_end_stream());

        body.frame().await.unwrap().unwrap();
        assert!(body.is_end_stream());
        assert!(body.frame().await.is_none());
    }

    #[tokio::test]
    async fn retry_clone_replays_full_body() {
        let input = pattern(100 * KIB);
        let mut body = chunked(input.clone()).into_inner();
        drain(&mut body).await;

        let mut retry = body.try_clone().expect("chunked bodies are retryable");

        assert_eq!(drain(&mut retry).await.concat(), input.to_vec());
    }

    async fn open_with(
        bytes: &[u8],
    ) -> (
        crate::test_utils::filesys::files::TempFile,
        Arc<std::fs::File>,
    ) {
        let tf = temp_file_with(bytes).await;
        let file = std::fs::File::open(tf.file().path()).unwrap();
        (tf, Arc::new(file))
    }

    #[tokio::test]
    async fn file_range_streams_the_range_in_bounded_frames() {
        let input = pattern(100 * KIB);
        let (_tf, file) = open_with(&input).await;

        let mut body = file_range(file, 10, 70 * KIB as u64, ReadFailure::default()).into_inner();
        assert!(body.bytes().is_none());
        assert_eq!(body.content_length(), Some(70 * KIB as u64));

        let frames = drain(&mut body).await;

        assert_eq!(lens(&frames), vec![32 * KIB, 32 * KIB, 6 * KIB]);
        assert_eq!(frames.concat(), input[10..10 + 70 * KIB].to_vec());
    }

    #[tokio::test]
    async fn file_range_retry_restarts_at_the_offset() {
        let input = pattern(100 * KIB);
        let (_tf, file) = open_with(&input).await;
        let mut body = file_range(file, 5, 40 * KIB as u64, ReadFailure::default()).into_inner();
        drain(&mut body).await;

        let mut retry = body.try_clone().expect("file ranges are retryable");

        assert_eq!(
            drain(&mut retry).await.concat(),
            input[5..5 + 40 * KIB].to_vec()
        );
    }

    #[tokio::test]
    async fn file_range_past_end_of_file_errors_and_records_the_failure() {
        let (_tf, file) = open_with(&pattern(40 * KIB)).await;
        let failure = ReadFailure::default();
        let mut body = file_range(file, 0, 64 * KIB as u64, failure.clone()).into_inner();

        let first = body.frame().await.unwrap().unwrap().into_data().unwrap();
        assert_eq!(first.len(), FRAME_SIZE);
        let err = body.frame().await.unwrap().unwrap_err();

        assert!(err.to_string().contains("file ended"), "got {err}");
        assert!(body.frame().await.is_none());
        let recorded = failure.take().expect("read failure is recorded");
        assert_eq!(recorded.kind(), io::ErrorKind::UnexpectedEof);
        assert!(failure.take().is_none());
    }

    #[tokio::test]
    async fn read_exact_at_reads_without_moving_a_cursor() {
        let (_tf, file) = open_with(b"0123456789").await;
        let mut buf = [0u8; 3];

        read_exact_at(&file, &mut buf, 4).unwrap();
        assert_eq!(&buf, b"456");
        read_exact_at(&file, &mut buf, 0).unwrap();
        assert_eq!(&buf, b"012");
        let err = read_exact_at(&file, &mut buf, 8).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }
}
