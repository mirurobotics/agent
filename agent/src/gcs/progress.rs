// standard crates
use std::future::Future;
use std::time::Duration;

// internal crates
use crate::gcs::{errors::ConnectionErr, GcsErr, Object};
use crate::trace;

// external crates
use bytes::Bytes;
use google_cloud_storage::streaming_source::{Seek, SizeHint, StreamingSource};
use tokio::sync::watch;

/// Largest chunk handed to the HTTP stack. Hyper pulls the next frame only
/// after buffering the previous one, so small frames keep progress ticking on
/// slow links (the SDK's file source reads 256 KiB at a time).
pub(super) const FRAME_SIZE: usize = 32 * 1024;

/// Upload progress as seen by the HTTP stack: cumulative bytes yielded and
/// whether the inner source has ended.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Progress {
    pub(super) sent: u64,
    pub(super) eof: bool,
}

/// How long an upload may go without progress: `idle` while the body is
/// streaming, `finalize` after its last byte (send buffers drain with no ticks
/// before the server answers).
#[derive(Clone, Copy)]
pub(super) struct Limits {
    pub(super) idle: Duration,
    pub(super) finalize: Duration,
}

/// Wraps an upload source, re-chunking it into [`FRAME_SIZE`] frames and
/// publishing [`Progress`] as the HTTP stack pulls them.
pub(super) struct ProgressSource<S> {
    inner: S,
    pending: Bytes,
    progress: watch::Sender<Progress>,
}

impl<S> ProgressSource<S> {
    pub(super) fn new(inner: S, progress: watch::Sender<Progress>) -> Self {
        Self {
            inner,
            pending: Bytes::new(),
            progress,
        }
    }
}

impl<S: StreamingSource + Send + Sync> StreamingSource for ProgressSource<S> {
    type Error = S::Error;

    async fn next(&mut self) -> Option<Result<Bytes, Self::Error>> {
        while self.pending.is_empty() {
            match self.inner.next().await {
                Some(Ok(chunk)) => self.pending = chunk,
                Some(Err(e)) => return Some(Err(e)),
                None => {
                    self.progress.send_if_modified(|p| !std::mem::replace(&mut p.eof, true));
                    return None;
                }
            }
        }
        let frame = self.pending.split_to(FRAME_SIZE.min(self.pending.len()));
        let len = frame.len() as u64;
        self.progress.send_modify(|p| p.sent += len);
        Some(Ok(frame))
    }

    async fn size_hint(&self) -> Result<SizeHint, Self::Error> {
        self.inner.size_hint().await
    }
}

impl<S: Seek> Seek for ProgressSource<S> {
    type Error = S::Error;

    /// Rewinds for an SDK (re)try. Not progress: a retry loop against a dead
    /// network must not keep the watchdog alive. Leaving end of body returns
    /// the watchdog to the idle bound.
    fn seek(&mut self, offset: u64) -> impl Future<Output = Result<(), Self::Error>> + Send {
        self.pending.clear();
        self.progress.send_if_modified(|p| std::mem::replace(&mut p.eof, false));
        self.inner.seek(offset)
    }
}

/// Runs `fut` until it completes or `progress` stalls past `limits`, in which
/// case the call is abandoned with a retryable [`GcsErr::ConnectionErr`].
pub(super) async fn watch_progress<T>(
    fut: impl Future<Output = Result<T, GcsErr>>,
    mut progress: watch::Receiver<Progress>,
    limits: Limits,
    obj: &Object,
) -> Result<T, GcsErr> {
    let bound = tokio::select! {
        biased;
        r = fut => return r,
        bound = idle_elapsed(&mut progress, limits) => bound,
    };
    let sent = progress.borrow().sent;
    Err(GcsErr::ConnectionErr(ConnectionErr {
        object: obj.clone(),
        msg: format!("no upload progress for {}s ({sent} bytes sent)", bound.as_secs()),
        trace: trace!(),
    }))
}

/// Resolves with the bound that elapsed once no progress arrives within it.
async fn idle_elapsed(progress: &mut watch::Receiver<Progress>, limits: Limits) -> Duration {
    loop {
        let bound = if progress.borrow_and_update().eof {
            limits.finalize
        } else {
            limits.idle
        };
        match tokio::time::timeout(bound, progress.changed()).await {
            Ok(Ok(())) => {}
            // The source was dropped: no more ticks can arrive, so wait out the
            // window instead of spinning.
            Ok(Err(_)) => {
                tokio::time::sleep(bound).await;
                return bound;
            }
            Err(_) => return bound,
        }
    }
}
