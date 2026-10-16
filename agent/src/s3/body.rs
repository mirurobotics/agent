// standard crates
use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context, Poll};

// external crates
use aws_sdk_s3::primitives::ByteStream;
use aws_smithy_types::body::SdkBody;
use bytes::Bytes;
use http_body::{Body, Frame, SizeHint};

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
