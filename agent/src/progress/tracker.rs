// standard crates
use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::progress::counter::Progress;
use crate::progress::format::{duration, mib, position, rate};
use crate::progress::report::Intervals;

// external crates
use tokio::time::{Instant, MissedTickBehavior};
use tracing::{info, warn};

/// How often [`track`] logs a running transfer.
pub const REPORT_INTERVAL: Duration = Duration::from_secs(30);

/// Runs `work` with a fresh [`Progress`] for a transfer of `total` bytes and
/// logs it every [`REPORT_INTERVAL`] until `work` returns, then logs the
/// outcome. If this future is dropped before `work` returns (e.g. by a
/// deadline), how far it got is logged as cancelled.
///
/// `unit` is how many bytes `work` counts at a time when it only counts whole
/// chunks (e.g. S3 parts), or 0 when it counts bytes as they flow. An interval
/// without progress logs a warning, so a stalled transfer shows up while it is
/// stalled; with a `unit`, the same interval is expected on a slow link and
/// logs at info instead.
pub async fn track<T, E, F, Fut>(label: String, total: u64, unit: u64, work: F) -> Result<T, E>
where
    F: FnOnce(Arc<Progress>) -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let progress = Arc::new(Progress::new(total));
    let ending = Ending {
        label,
        progress: progress.clone(),
        started: Instant::now(),
        logged: false,
    };
    let mut intervals = Intervals::new(total, unit, REPORT_INTERVAL);
    // Boxed so a caller's async state holds a pointer instead of inlining the
    // transfer's (deep SDK) future, which overflows rustc's layout query depth.
    let mut work = Box::pin(work(progress.clone()));
    let mut ticker = tokio::time::interval_at(ending.started + REPORT_INTERVAL, REPORT_INTERVAL);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let result = loop {
        tokio::select! {
            result = &mut work => break result,
            _ = ticker.tick() => {
                let report = intervals.report(progress.sent(), ending.started.elapsed());
                if report.healthy {
                    info!("{}: {}", ending.label, report.message);
                } else {
                    warn!("{}: {}", ending.label, report.message);
                }
            }
        }
    };
    ending.log(if result.is_ok() {
        Outcome::Completed
    } else {
        Outcome::Failed
    });
    result
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Completed,
    Failed,
    Cancelled,
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Outcome::Completed => "completed",
            Outcome::Failed => "failed",
            Outcome::Cancelled => "cancelled",
        })
    }
}

/// Logs how a tracked transfer ended: once through [`Self::log`], or as
/// cancelled when dropped without it.
struct Ending {
    label: String,
    progress: Arc<Progress>,
    started: Instant,
    logged: bool,
}

impl Ending {
    fn log(mut self, outcome: Outcome) {
        self.logged = true;
        self.write(outcome);
    }

    fn write(&self, outcome: Outcome) {
        let progress = &self.progress;
        let message = summary(
            outcome,
            progress.total(),
            progress.sent(),
            self.started.elapsed(),
        );
        match outcome {
            Outcome::Completed => info!("{}: {message}", self.label),
            Outcome::Failed | Outcome::Cancelled => warn!("{}: {message}", self.label),
        }
    }
}

impl Drop for Ending {
    fn drop(&mut self) {
        if !self.logged {
            self.write(Outcome::Cancelled);
        }
    }
}

fn summary(outcome: Outcome, total: u64, sent: u64, elapsed: Duration) -> String {
    format!(
        "{outcome} after {}; {}, {}/s average",
        duration(elapsed.as_secs()),
        position(total, sent),
        mib(rate(sent, elapsed))
    )
}

#[cfg(test)]
mod tests {
    // internal crates
    use super::*;

    const MIB: u64 = 1024 * 1024;

    mod summary {
        use super::*;

        #[test]
        fn reports_outcome_position_and_rate() {
            assert_eq!(
                summary(Outcome::Failed, 4 * MIB, 2 * MIB, Duration::from_secs(2)),
                "failed after 2s; 2.0 MiB of 4.0 MiB (50.0%) sent, 1.0 MiB/s average"
            );
        }

        #[test]
        fn zero_elapsed_has_zero_rate() {
            assert_eq!(
                summary(Outcome::Completed, 0, 0, Duration::ZERO),
                "completed after 0s; 0.0 MiB sent, 0.0 MiB/s average"
            );
        }
    }

    mod track {
        use super::*;

        #[tokio::test]
        async fn returns_the_work_result_with_its_progress() {
            let result = track("test".to_string(), 10, 0, |progress| async move {
                progress.add(10);
                Ok::<_, ()>((progress.total(), progress.sent()))
            })
            .await;
            assert_eq!(result, Ok((10, 10)));
        }

        // Covers the ticker wiring; `intervals` checks the report sequence.
        #[tokio::test(start_paused = true)]
        async fn reports_while_the_work_runs() {
            let result = track("test".to_string(), 10 * MIB, 0, |progress| async move {
                progress.add(MIB);
                tokio::time::sleep(Duration::from_secs(61)).await;
                Err::<(), _>("failed")
            })
            .await;
            assert_eq!(result, Err("failed"));
        }

        #[tokio::test(start_paused = true)]
        async fn dropping_before_the_work_returns_cancels_it() {
            let tracked = track("test".to_string(), 10, 0, |_| {
                std::future::pending::<Result<(), ()>>()
            });
            let timed_out = tokio::time::timeout(Duration::from_secs(1), tracked).await;
            assert!(timed_out.is_err());
        }
    }
}
