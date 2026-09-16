//! OS service-manager integration.
//!
//! The portable pieces live here: [`StopSignal`] relays a service-manager stop
//! request into the shutdown future `app::run` awaits, and [`RunOutcome`] is
//! what the agent body reports back so the service manager can be told whether
//! the service exited cleanly. [`windows`] holds the Service Control Manager
//! plumbing and compiles only on Windows.

// standard crates
use std::future::Future;

// external crates
use tokio::sync::watch;

pub mod errors;
#[cfg(windows)]
pub mod windows;

/// How the agent body finished, mapped to a service exit code by the OS layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    /// The body returned normally (including a requested shutdown).
    Completed,
    /// The body bailed out early or returned an error.
    Failed,
}

/// One-shot, multi-waiter stop relay.
///
/// [`trigger`](Self::trigger) may be called from any thread, with or without
/// a tokio runtime, is idempotent, and wakes every [`wait`](Self::wait) future
/// created before or after the call. A `wait()` future created after the
/// signal has been triggered resolves immediately.
#[derive(Debug, Clone)]
pub struct StopSignal {
    tx: watch::Sender<bool>,
}

impl Default for StopSignal {
    fn default() -> Self {
        Self::new()
    }
}

impl StopSignal {
    /// Creates an untriggered signal.
    pub fn new() -> Self {
        let (tx, _rx) = watch::channel(false);
        Self { tx }
    }

    /// Marks the signal as triggered and wakes every waiter. Safe to call
    /// repeatedly and from a non-tokio thread.
    pub fn trigger(&self) {
        self.tx.send_replace(true);
    }

    /// Whether [`trigger`](Self::trigger) has been called.
    pub fn is_triggered(&self) -> bool {
        *self.tx.borrow()
    }

    /// Returns a future that resolves once the signal is triggered, or once
    /// every `StopSignal` handle has been dropped (a stop that can no longer
    /// arrive). The future owns its receiver so it is `'static` and can be
    /// handed to `app::run`; it checks the current value first, so it
    /// resolves at once when the signal is already triggered.
    pub fn wait(&self) -> impl Future<Output = ()> + Send + 'static {
        let mut rx = self.tx.subscribe();
        async move {
            let _ = rx.wait_for(|triggered| *triggered).await;
        }
    }
}
