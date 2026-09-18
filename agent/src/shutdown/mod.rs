//! Process-wide shutdown types.
//!
//! [`Latch`] is the process-wide stop latch: one `trigger()` completes every
//! current and future `wait()` that `run_agent` awaits (activation, upgrade
//! reconcile, and the long-running agent). [`RunOutcome`] is how that body
//! finished; the Windows SCM maps it to a service exit code.

// standard crates
use std::future::Future;

// external crates
use tokio::sync::watch;

/// How the agent body finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    /// The body returned normally (including a requested shutdown).
    Completed,
    /// The body bailed out early or returned an error.
    Failed,
}

/// One-shot, multi-waiter latch.
///
/// [`trigger`](Self::trigger) may be called from any thread, with or without
/// a tokio runtime, is idempotent, and wakes every [`wait`](Self::wait) future
/// created before or after the call. A `wait()` future created after the
/// latch has been triggered resolves immediately.
#[derive(Debug, Clone)]
pub struct Latch {
    tx: watch::Sender<bool>,
}

impl Default for Latch {
    fn default() -> Self {
        Self::new()
    }
}

impl Latch {
    /// Creates an untriggered latch.
    pub fn new() -> Self {
        let (tx, _rx) = watch::channel(false);
        Self { tx }
    }

    /// Marks the latch as triggered and wakes every waiter. Safe to call
    /// repeatedly and from a non-tokio thread.
    pub fn trigger(&self) {
        self.tx.send_replace(true);
    }

    /// Whether [`trigger`](Self::trigger) has been called.
    pub fn is_triggered(&self) -> bool {
        *self.tx.borrow()
    }

    /// Returns a future that resolves once the signal is triggered, or once
    /// every `Latch` handle has been dropped (a stop that can no longer
    /// arrive). The future owns its receiver so it is `'static` and can be
    /// handed to `app::run`; it checks the current value first, so it
    /// resolves at once when the latch is already triggered.
    pub fn wait(&self) -> impl Future<Output = ()> + Send + 'static {
        let mut rx = self.tx.subscribe();
        async move {
            let _ = rx.wait_for(|triggered| *triggered).await;
        }
    }
}
