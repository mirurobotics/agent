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
        msg: format!(
            "no upload progress for {}s ({sent} bytes sent)",
            bound.as_secs()
        ),
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

#[cfg(test)]
mod tests {
    // standard crates
    use std::future::pending;

    // internal crates
    use super::*;
    use crate::gcs::errors::RequestFailedErr;
    use miru_agent::errors::Error;

    // external crates
    use google_cloud_storage::streaming_source::{BytesSource, Payload};
    use tokio::time::Instant;

    const KIB: usize = 1024;

    /// `finalize` is deliberately not the production 90 s, so a seek at 30 s
    /// (idle window ends at 90 s) is distinguishable from the finalize bound.
    const LIMITS: Limits = Limits {
        idle: Duration::from_secs(60),
        finalize: Duration::from_secs(120),
    };

    fn obj() -> Object {
        Object {
            bucket: "test-bucket".to_string(),
            key: "k".to_string(),
        }
    }

    /// Bytes with a period (251) that no frame size divides, so a dropped or
    /// reordered frame cannot go unnoticed.
    fn pattern(len: usize) -> Bytes {
        Bytes::from((0..len).map(|i| (i % 251) as u8).collect::<Vec<u8>>())
    }

    async fn drain<S: StreamingSource + Send + Sync>(src: &mut ProgressSource<S>) -> Vec<Bytes> {
        let mut frames = Vec::new();
        while let Some(frame) = src.next().await {
            frames.push(frame.unwrap());
        }
        frames
    }

    fn lens(frames: &[Bytes]) -> Vec<usize> {
        frames.iter().map(Bytes::len).collect()
    }

    pub mod source {
        use super::*;

        /// Never yields data: the first read fails.
        struct FailingSource;

        impl StreamingSource for FailingSource {
            type Error = std::io::Error;

            async fn next(&mut self) -> Option<Result<Bytes, Self::Error>> {
                Some(Err(std::io::Error::other("read failed")))
            }
        }

        #[tokio::test]
        async fn rechunks_into_bounded_frames() {
            let input = pattern(100 * KIB);
            let (tx, _rx) = watch::channel(Progress::default());
            let mut src = ProgressSource::new(Payload::from(input.clone()), tx);

            let frames = drain(&mut src).await;

            assert_eq!(lens(&frames), vec![32 * KIB, 32 * KIB, 32 * KIB, 4 * KIB]);
            assert_eq!(frames.concat(), input.to_vec());
        }

        #[tokio::test]
        async fn preserves_bytes_across_inner_chunks() {
            let first = pattern(40 * KIB);
            let second = Bytes::from(vec![7u8; 10 * KIB]);
            let (tx, _rx) = watch::channel(Progress::default());
            let mut src = ProgressSource::new(
                Payload::from(vec![first.clone(), second.clone()]),
                tx,
            );

            let frames = drain(&mut src).await;

            // A frame never spans two inner chunks.
            assert_eq!(lens(&frames), vec![32 * KIB, 8 * KIB, 10 * KIB]);
            assert_eq!(frames.concat(), [first, second].concat());
        }

        #[tokio::test]
        async fn forwards_exact_size_hint() {
            let (tx, _rx) = watch::channel(Progress::default());
            let src = ProgressSource::new(Payload::from(pattern(100 * KIB)), tx);

            let hint = src.size_hint().await.unwrap();

            assert_eq!(hint.exact(), Some(100 * KIB as u64));
        }

        #[tokio::test]
        async fn inner_error_passes_through() {
            let (tx, rx) = watch::channel(Progress::default());
            let mut src = ProgressSource::new(FailingSource, tx);

            let next = src.next().await;

            assert!(matches!(next, Some(Err(_))), "got {next:?}");
            assert_eq!(*rx.borrow(), Progress::default());
        }

        #[tokio::test]
        async fn seek_discards_pending_and_rewinds() {
            let input = pattern(100 * KIB);
            let (tx, _rx) = watch::channel(Progress::default());
            let mut src = ProgressSource::new(Payload::from(input.clone()), tx);
            // One frame leaves the rest of the inner chunk pending.
            src.next().await.unwrap().unwrap();

            src.seek(0).await.unwrap();
            let frames = drain(&mut src).await;

            assert_eq!(frames.concat(), input.to_vec());
        }

        #[tokio::test]
        async fn progress_tracks_bytes_and_eof() {
            let total = 100 * KIB as u64;
            let (tx, rx) = watch::channel(Progress::default());
            let mut src = ProgressSource::new(Payload::from(pattern(100 * KIB)), tx);

            drain(&mut src).await;
            assert_eq!(
                *rx.borrow(),
                Progress {
                    sent: total,
                    eof: true,
                }
            );

            // A rewind leaves the end of body but is not progress.
            src.seek(0).await.unwrap();
            assert_eq!(
                *rx.borrow(),
                Progress {
                    sent: total,
                    eof: false,
                }
            );
        }
    }

    pub mod watchdog {
        use super::*;

        fn fresh() -> (watch::Sender<Progress>, watch::Receiver<Progress>) {
            watch::channel(Progress::default())
        }

        type Source = ProgressSource<Payload<BytesSource>>;

        /// A source drained to its end, which puts the watchdog on the finalize
        /// bound. `tx` stays alive as long as the returned source.
        async fn ended_source(tx: watch::Sender<Progress>) -> Source {
            let mut src = ProgressSource::new(Payload::from(pattern(KIB)), tx);
            drain(&mut src).await;
            src
        }

        fn stalled() -> impl Future<Output = Result<(), GcsErr>> {
            pending()
        }

        fn assert_stall_err(err: &GcsErr) {
            assert!(matches!(err, GcsErr::ConnectionErr(_)), "got {err:?}");
            assert!(err.is_network_conn_err());
        }

        #[tokio::test(start_paused = true)]
        async fn fires_after_idle() {
            let (_tx, rx) = fresh();
            let start = Instant::now();

            let err = watch_progress(stalled(), rx, LIMITS, &obj())
                .await
                .unwrap_err();

            assert_stall_err(&err);
            assert_eq!(start.elapsed(), LIMITS.idle);
        }

        #[tokio::test(start_paused = true)]
        async fn keeps_waiting_while_progress_arrives() {
            let (tx, rx) = fresh();
            // A tick every 30 s keeps every 60 s idle window satisfied.
            let _ticker = tokio::spawn(async move {
                for _ in 0..5 {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    tx.send_modify(|p| p.sent += 1);
                }
                tx
            });
            let start = Instant::now();
            let slow = async {
                tokio::time::sleep(Duration::from_secs(150)).await;
                Ok::<_, GcsErr>(42)
            };

            let out = watch_progress(slow, rx, LIMITS, &obj()).await.unwrap();

            assert_eq!(out, 42);
            assert_eq!(start.elapsed(), Duration::from_secs(150));
        }

        #[tokio::test(start_paused = true)]
        async fn returns_inner_result() {
            let (_tx, rx) = fresh();
            let failed = async {
                Err::<(), _>(GcsErr::RequestFailedErr(RequestFailedErr {
                    operation: "put_object".to_string(),
                    object: obj(),
                    status: Some(418),
                    msg: "nope".to_string(),
                    trace: miru_agent::trace!(),
                }))
            };

            let err = watch_progress(failed, rx, LIMITS, &obj())
                .await
                .unwrap_err();

            match err {
                GcsErr::RequestFailedErr(e) => assert_eq!(e.status, Some(418)),
                other => panic!("expected RequestFailedErr, got {other:?}"),
            }
        }

        #[tokio::test(start_paused = true)]
        async fn dropped_sender_still_waits_full_window() {
            let (tx, rx) = fresh();
            drop(tx);
            let start = Instant::now();

            let err = watch_progress(stalled(), rx, LIMITS, &obj())
                .await
                .unwrap_err();

            assert_stall_err(&err);
            assert_eq!(start.elapsed(), LIMITS.idle);
        }

        #[tokio::test(start_paused = true)]
        async fn eof_switches_to_finalize_bound() {
            let (tx, rx) = fresh();
            let _src = ended_source(tx).await;
            let start = Instant::now();

            let err = watch_progress(stalled(), rx, LIMITS, &obj())
                .await
                .unwrap_err();

            assert_stall_err(&err);
            assert_eq!(start.elapsed(), LIMITS.finalize);
        }

        #[tokio::test(start_paused = true)]
        async fn seek_after_eof_restores_idle_bound() {
            let (tx, rx) = fresh();
            let mut src = ended_source(tx).await;
            let start = Instant::now();
            // The returned source keeps the sender alive in the join handle.
            let _rewind = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(30)).await;
                src.seek(0).await.unwrap();
                src
            });

            let err = watch_progress(stalled(), rx, LIMITS, &obj())
                .await
                .unwrap_err();

            assert_stall_err(&err);
            assert_eq!(start.elapsed(), Duration::from_secs(30) + LIMITS.idle);
        }
    }
}
