//! Retry cooldowns for failed deployments.

// standard crates
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// internal crates
use crate::clock::Clock;
use crate::models::deployment::DeploymentID;

/// When each failed deployment may be retried, as monotonic deadlines, so a
/// system clock step can neither cut a cooldown short nor stretch it.
///
/// Cooldowns live in memory only. After a restart every deployment may retry
/// right away; the attempt count saved with the deployment still sets the
/// next backoff. Clones share the same cooldowns.
#[derive(Clone, Debug, Default)]
pub struct Cooldowns {
    clock: Clock,
    ends: Arc<Mutex<HashMap<DeploymentID, Instant>>>,
}

impl Cooldowns {
    pub fn new(clock: Clock) -> Self {
        Self {
            clock,
            ends: Arc::default(),
        }
    }

    /// Starts a cooldown of `wait` for deployment `id`, replacing any current
    /// one. A zero `wait` clears it.
    pub fn start(&self, id: &str, wait: Duration) {
        if wait.is_zero() {
            self.clear(id);
            return;
        }
        let ends = self.clock.monotonic_now() + wait;
        self.lock().insert(id.to_string(), ends);
    }

    /// Ends deployment `id`'s cooldown, if it has one.
    pub fn clear(&self, id: &str) {
        self.lock().remove(id);
    }

    /// How much of deployment `id`'s cooldown is left, or `None` if it is not
    /// cooling down.
    pub fn remaining(&self, id: &str) -> Option<Duration> {
        let mut ends = self.lock();
        let remaining = ends
            .get(id)?
            .saturating_duration_since(self.clock.monotonic_now());
        if remaining.is_zero() {
            ends.remove(id);
            return None;
        }
        Some(remaining)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<DeploymentID, Instant>> {
        self.ends.lock().unwrap_or_else(|e| e.into_inner())
    }
}
