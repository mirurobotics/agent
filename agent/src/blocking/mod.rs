//! Runs blocking work (disk syncs, hashing, key generation) on Tokio's
//! blocking pool, so it never stalls an async worker thread.

/// Runs `op` on the blocking pool and returns its result. A panic in `op` is
/// re-raised here with its original payload, as if `op` had run inline.
pub async fn run<T: Send + 'static>(op: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(op)
        .await
        .unwrap_or_else(|e| std::panic::resume_unwind(e.into_panic()))
}
