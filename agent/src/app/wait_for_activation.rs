// standard crates
use std::future::Future;
use std::time::Duration;

// internal crates
use crate::storage::{self, Layout};

// external crates
use tokio::pin;
use tracing::info;

/// Outcome of a `wait_for_activation` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitOutcome {
    /// The auth keys appeared on disk and the agent should proceed.
    Activated,
    /// A shutdown signal was received before activation completed.
    ShutdownRequested,
}

/// Block until the device is activated (auth keys exist on disk) or a shutdown
/// signal is received. Polls `storage::assert_activated` once per cycle using
/// the injected `sleep_fn` (1-second cycles in production), throttling logs
/// via [`should_log`]. The shutdown future is consumed once and races every
/// poll cycle via `tokio::select!`.
///
/// `biased` ordering on the select makes the shutdown branch win on a tie, so
/// a SIGTERM that arrives in the same poll iteration as the keys appearing
/// still results in a clean shutdown.
pub async fn wait_for_activation<F, Fut, S>(
    layout: &Layout,
    sleep_fn: F,
    shutdown: S,
) -> WaitOutcome
where
    F: Fn(Duration) -> Fut,
    Fut: Future<Output = ()> + Send,
    S: Future<Output = ()> + Send,
{
    // Fast path: already activated. Avoid the "not yet activated" log entirely
    // so a normal start-up on an already-provisioned device is silent here.
    if storage::assert_activated(layout).await.is_ok() {
        info!("Device activated; starting agent.");
        return WaitOutcome::Activated;
    }

    info!("Device is not yet activated; waiting for provisioning...");

    pin!(shutdown);
    let mut cycle: u64 = 1;

    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => {
                info!("Shutdown received while waiting for activation");
                return WaitOutcome::ShutdownRequested;
            }
            _ = sleep_fn(Duration::from_secs(1)) => {
                if storage::assert_activated(layout).await.is_ok() {
                    info!("Device activated; starting agent.");
                    return WaitOutcome::Activated;
                }
                if should_log(cycle) {
                    info!("Still waiting for activation (waited {cycle}s)...");
                }
                cycle = cycle.saturating_add(1);
            }
        }
    }
}

/// Returns `true` if the wait-for-activation loop should emit a heartbeat log
/// at the given 1-based cycle counter (i.e. seconds elapsed since the
/// initial "not yet activated" log).
///
/// Logs fire at cycles 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, then every
/// 1024 cycles thereafter. Cycle 0's first log is emitted by the caller
/// before entering the loop, so this fn returns `false` for 0; cycle 1 is
/// intentionally silent — `1.is_power_of_two()` is `true` but the spec
/// excludes it.
pub fn should_log(cycle: u64) -> bool {
    if cycle < 2 {
        return false;
    }
    if cycle <= 1024 {
        return cycle.is_power_of_two();
    }
    cycle % 1024 == 0
}

#[cfg(test)]
mod tests {
    use super::should_log;

    #[test]
    fn cycle_zero_is_silent_caller_logs_first_miss() {
        // Cycle 0 logging happens outside the loop in `wait_for_activation`,
        // so `should_log(0) == false` — otherwise we'd double-log.
        assert!(!should_log(0));
    }

    #[test]
    fn cycle_one_is_silent() {
        assert!(!should_log(1));
    }

    #[test]
    fn powers_of_two_through_1024_log() {
        for n in [2u64, 4, 8, 16, 32, 64, 128, 256, 512, 1024] {
            assert!(should_log(n), "expected log at cycle {n}");
        }
    }

    #[test]
    fn multiples_of_1024_above_cap_log() {
        for n in [2048u64, 3072, 4096, 5120, 102_400, 1_048_576] {
            assert!(should_log(n), "expected heartbeat at cycle {n}");
        }
    }

    #[test]
    fn off_cycle_values_are_silent() {
        for n in [
            3u64, 5, 7, 9, 17, 31, 63, 127, 255, 511, 513, 1023, 1025, 2047, 3071,
        ] {
            assert!(!should_log(n), "expected silent at cycle {n}");
        }
    }

    #[test]
    fn powers_of_two_above_1024_only_log_when_also_multiple_of_1024() {
        // 2048, 4096, 8192 are all multiples of 1024 AND powers of 2 — they
        // log via the heartbeat branch. But e.g. cycle 2049 is silent even
        // though we're past the cap.
        assert!(!should_log(2049));
        assert!(!should_log(4097));
    }
}
