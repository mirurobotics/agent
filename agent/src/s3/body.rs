// standard crates
use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context, Poll};

// external crates
use aws_sdk_s3::primitives::ByteStream;
use aws_smithy_types::body::SdkBody;
use bytes::Bytes;
use http_body::{Body, Frame, SizeHint};

const FRAME_SIZE: usize = 32 * 1024;

/// A request body sent in frames, so stalled-stream protection sees progress
/// as each frame goes out. A body of one frame stays a plain buffer. Retries
/// replay the body from a cheap `Bytes` clone.
pub(super) fn framed(body: Vec<u8>) -> ByteStream {
    let body = Bytes::from(body);
    if body.len() <= FRAME_SIZE {
        return ByteStream::from(body);
    }
    ByteStream::new(SdkBody::retryable(move || {
        SdkBody::from_body_1_x(FramedBytes {
            remaining: body.clone(),
        })
    }))
}

struct FramedBytes {
    remaining: Bytes,
}

impl Body for FramedBytes {
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

    // Exact, so the SDK can send a Content-Length and, on AWS, the checksum
    // trailer.
    fn size_hint(&self) -> SizeHint {
        SizeHint::with_exact(self.remaining.len() as u64)
    }
}

#[cfg(test)]
mod tests {
    // internal crates
    use super::*;

    // external crates
    use http_body_util::BodyExt;

    const KIB: usize = 1024;

    /// Bytes with a period (251) that no frame size divides, so a dropped or
    /// reordered frame cannot go unnoticed.
    fn pattern(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    /// Polls `body` to its end, returning each data frame.
    async fn drain(body: &mut SdkBody) -> Vec<Bytes> {
        let mut frames = Vec::new();
        while let Some(frame) = body.frame().await {
            frames.push(frame.unwrap().into_data().unwrap());
        }
        frames
    }

    #[test]
    fn small_body_stays_in_memory() {
        let input = pattern(FRAME_SIZE);

        let body = framed(input.clone()).into_inner();

        assert_eq!(body.bytes(), Some(&input[..]));
    }

    #[tokio::test]
    async fn large_body_yields_bounded_frames() {
        let input = pattern(100 * KIB);
        let mut body = framed(input.clone()).into_inner();

        let frames = drain(&mut body).await;

        let lens: Vec<usize> = frames.iter().map(Bytes::len).collect();
        assert_eq!(lens, vec![32 * KIB, 32 * KIB, 32 * KIB, 4 * KIB]);
        assert_eq!(frames.concat(), input);
    }

    #[tokio::test]
    async fn size_hint_is_exact_and_end_stream_tracks_remaining() {
        let mut body = FramedBytes {
            remaining: Bytes::from(pattern(40 * KIB)),
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
        let mut body = framed(input.clone()).into_inner();
        drain(&mut body).await;

        let mut retry = body.try_clone().expect("framed bodies are retryable");

        assert_eq!(drain(&mut retry).await.concat(), input);
    }
}
