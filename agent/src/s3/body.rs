// standard crates
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{ready, Context, Poll};

// internal crates
use crate::filesys::files;

// external crates
use aws_sdk_s3::primitives::ByteStream;
use aws_smithy_types::body::SdkBody;
use bytes::Bytes;
use http_body::{Body, Frame, SizeHint};
use tokio::task::JoinHandle;

/// How much of a file a request body, or a GCS hash pass, reads at a time.
pub(super) const FRAME_SIZE: usize = 32 * 1024;

/// A request body that streams `len` bytes of `file` from `offset` in
/// [`FRAME_SIZE`] frames: at most one frame is in memory, and stalled-stream
/// protection sees progress as each frame goes out. Reads are positional, so
/// a retry restarts at `offset` on the same opened file.
pub(super) fn file_range(file: Arc<std::fs::File>, offset: u64, len: u64) -> ByteStream {
    ByteStream::new(SdkBody::retryable(move || {
        SdkBody::from_body_1_x(FileRange {
            file: file.clone(),
            offset,
            remaining: len,
            reading: None,
        })
    }))
}

struct FileRange {
    file: Arc<std::fs::File>,
    offset: u64,
    remaining: u64,
    /// The read of the next frame, while it runs.
    reading: Option<JoinHandle<io::Result<Vec<u8>>>>,
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
        let reading = this.reading.get_or_insert_with(|| {
            let (file, offset) = (this.file.clone(), this.offset);
            let mut buf = vec![0u8; this.remaining.min(FRAME_SIZE as u64) as usize];
            tokio::task::spawn_blocking(move || {
                files::read_exact_at(&file, &mut buf, offset).map(|()| buf)
            })
        });
        let read = ready!(Pin::new(reading).poll(cx));
        this.reading = None;
        let buf = read.unwrap_or_else(|e| Err(io::Error::other(e)))?;
        this.offset += buf.len() as u64;
        this.remaining -= buf.len() as u64;
        Poll::Ready(Some(Ok(Frame::data(Bytes::from(buf)))))
    }

    fn is_end_stream(&self) -> bool {
        self.remaining == 0
    }

    // Exact, so the SDK can send a Content-Length and, on AWS, the checksum
    // trailer.
    fn size_hint(&self) -> SizeHint {
        SizeHint::with_exact(self.remaining)
    }
}

#[cfg(test)]
mod tests {
    // internal crates
    use super::*;
    use crate::filesys::path::PathExt;
    use crate::s3::store::tests::temp_file_with;
    use crate::test_utils::filesys::files::TempFile;

    // external crates
    use http_body_util::BodyExt;

    const KIB: usize = 1024;

    /// Bytes with a period (251) that no frame size divides, so a dropped or
    /// reordered frame cannot go unnoticed.
    fn pattern(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    /// A temp file holding `bytes`, opened for reading. Keep the guard alive
    /// while the handle is used.
    async fn open_with(bytes: &[u8]) -> (TempFile, Arc<std::fs::File>) {
        let tf = temp_file_with(bytes).await;
        let file = std::fs::File::open(tf.path()).unwrap();
        (tf, Arc::new(file))
    }

    /// Polls `body` to its end, returning each data frame.
    async fn drain(body: &mut SdkBody) -> Vec<Bytes> {
        let mut frames = Vec::new();
        while let Some(frame) = body.frame().await {
            frames.push(frame.unwrap().into_data().unwrap());
        }
        frames
    }

    #[tokio::test]
    async fn range_yields_bounded_frames() {
        let input = pattern(110 * KIB);
        let (_tf, file) = open_with(&input).await;
        let mut body = file_range(file, 10 * KIB as u64, 100 * KIB as u64).into_inner();

        let frames = drain(&mut body).await;

        let lens: Vec<usize> = frames.iter().map(Bytes::len).collect();
        assert_eq!(lens, vec![32 * KIB, 32 * KIB, 32 * KIB, 4 * KIB]);
        assert_eq!(frames.concat(), input[10 * KIB..]);
    }

    #[tokio::test]
    async fn size_hint_is_exact_and_end_stream_tracks_remaining() {
        let (_tf, file) = open_with(&pattern(40 * KIB)).await;
        let mut body = FileRange {
            file,
            offset: 0,
            remaining: 40 * KIB as u64,
            reading: None,
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
    async fn retry_clone_replays_the_range_from_its_offset() {
        let input = pattern(100 * KIB);
        let (_tf, file) = open_with(&input).await;
        let mut body = file_range(file, 5, 60 * KIB as u64).into_inner();
        drain(&mut body).await;

        let mut retry = body.try_clone().expect("file bodies are retryable");

        assert_eq!(drain(&mut retry).await.concat(), input[5..5 + 60 * KIB]);
    }

    #[tokio::test]
    async fn file_shorter_than_the_range_fails_the_body() {
        let (_tf, file) = open_with(&pattern(40 * KIB)).await;
        let mut body = file_range(file, 0, 64 * KIB as u64).into_inner();

        body.frame().await.unwrap().unwrap();
        let err = body.frame().await.unwrap().unwrap_err();

        assert!(err.to_string().contains("file ended"), "got {err}");
    }
}
