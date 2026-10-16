// standard crates
use std::fmt;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

// external crates
use tokio::time::{Instant, MissedTickBehavior};
use tracing::{info, warn};

/// How often [`track`] logs a running transfer.
pub const REPORT_INTERVAL: Duration = Duration::from_secs(30);

/// Bytes moved by a transfer of a fixed `total` size. Counts describe what the
/// transfer code has handed off, not what the remote end has acknowledged.
#[derive(Debug)]
pub struct Progress {
    total: u64,
    sent: AtomicU64,
}

impl Progress {
    pub fn new(total: u64) -> Self {
        Self {
            total,
            sent: AtomicU64::new(0),
        }
    }

    pub fn add(&self, bytes: u64) {
        self.sent.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Moves the position to `offset`, e.g. when the transfer resends from an
    /// earlier byte, and returns the previous position.
    pub fn seek_to(&self, offset: u64) -> u64 {
        self.sent.swap(offset, Ordering::Relaxed)
    }

    pub fn total(&self) -> u64 {
        self.total
    }

    pub fn sent(&self) -> u64 {
        self.sent.load(Ordering::Relaxed)
    }
}

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
    let mut intervals = Intervals {
        total,
        unit,
        interval: REPORT_INTERVAL,
        prev: 0,
    };
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

/// Turns one progress reading per interval into a report line, comparing each
/// reading with the one before it.
struct Intervals {
    total: u64,
    unit: u64,
    interval: Duration,
    prev: u64,
}

impl Intervals {
    fn report(&mut self, now: u64, elapsed: Duration) -> Report {
        let report = describe(
            self.total,
            self.unit,
            self.prev,
            now,
            self.interval,
            elapsed,
        );
        self.prev = now;
        report
    }
}

/// One periodic progress line. `healthy` is false when the interval made no
/// progress that it should have.
#[derive(Debug, PartialEq, Eq)]
struct Report {
    healthy: bool,
    message: String,
}

fn describe(
    total: u64,
    unit: u64,
    prev: u64,
    now: u64,
    interval: Duration,
    elapsed: Duration,
) -> Report {
    let interval_text = duration(interval.as_secs());
    if now <= prev {
        return stalled(total, unit, now, &interval_text, elapsed);
    }
    advancing(total, prev, now, interval, elapsed, &interval_text)
}

fn stalled(total: u64, unit: u64, now: u64, interval_text: &str, elapsed: Duration) -> Report {
    let position = position(total, now);
    let elapsed = duration(elapsed.as_secs());
    if unit > 0 {
        return Report {
            healthy: true,
            message: format!(
                "no {} chunk completed in the last {interval_text}; {position} after {elapsed}",
                mib(unit)
            ),
        };
    }
    Report {
        healthy: false,
        message: format!("no progress in the last {interval_text}; {position} after {elapsed}"),
    }
}

fn advancing(
    total: u64,
    prev: u64,
    now: u64,
    interval: Duration,
    elapsed: Duration,
    interval_text: &str,
) -> Report {
    let average = rate(now, elapsed);
    let mut message = format!(
        "{}; {}/s over the last {interval_text}, {}/s average",
        position(total, now),
        mib(rate(now - prev, interval)),
        mib(average)
    );
    if total > now && average > 0 {
        let eta = (total - now) / average;
        message.push_str(&format!("; about {} left", duration(eta)));
    }
    Report {
        healthy: true,
        message,
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

/// `{sent} of {total} ({pct}%) sent`, or just `{sent} sent` when `total` is 0.
fn position(total: u64, sent: u64) -> String {
    if total == 0 {
        return format!("{} sent", mib(sent));
    }
    let pct = sent as f64 * 100.0 / total as f64;
    format!("{} of {} ({pct:.1}%) sent", mib(sent), mib(total))
}

/// Bytes per second over `over`, or 0 for a zero-length span.
fn rate(bytes: u64, over: Duration) -> u64 {
    let millis = over.as_millis();
    if millis == 0 {
        return 0;
    }
    (u128::from(bytes) * 1000 / millis) as u64
}

fn mib(bytes: u64) -> String {
    format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
}

fn duration(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m{:02}s", secs / 60, secs % 60),
        _ => format!("{}h{:02}m", secs / 3600, secs % 3600 / 60),
    }
}

#[cfg(test)]
mod tests {
    // internal crates
    use super::*;

    const MIB: u64 = 1024 * 1024;
    const INTERVAL: Duration = Duration::from_secs(30);

    mod progress {
        use super::*;

        #[test]
        fn counts_bytes_and_seeks() {
            let progress = Progress::new(10);
            progress.add(4);
            progress.add(3);
            assert_eq!(progress.seek_to(2), 7);
            assert_eq!((progress.total(), progress.sent()), (10, 2));
        }
    }

    mod describe {
        use super::*;

        #[test]
        fn progressing_reports_rates_and_eta() {
            let report = describe(
                100 * MIB,
                0,
                10 * MIB,
                40 * MIB,
                INTERVAL,
                Duration::from_secs(40),
            );
            assert_eq!(
                report,
                Report {
                    healthy: true,
                    message: "40.0 MiB of 100.0 MiB (40.0%) sent; 1.0 MiB/s over the last 30s, \
                              1.0 MiB/s average; about 1m00s left"
                        .to_string(),
                }
            );
        }

        #[test]
        fn no_progress_is_a_stall() {
            let report = describe(
                100 * MIB,
                0,
                40 * MIB,
                40 * MIB,
                INTERVAL,
                Duration::from_secs(7200),
            );
            assert_eq!(
                report,
                Report {
                    healthy: false,
                    message: "no progress in the last 30s; 40.0 MiB of 100.0 MiB (40.0%) sent \
                              after 2h00m"
                        .to_string(),
                }
            );
        }

        #[test]
        fn moving_backwards_is_not_progress() {
            let report = describe(
                100 * MIB,
                0,
                40 * MIB,
                32 * MIB,
                INTERVAL,
                Duration::from_secs(60),
            );
            assert_eq!(
                report,
                Report {
                    healthy: false,
                    message: "no progress in the last 30s; 32.0 MiB of 100.0 MiB (32.0%) sent \
                              after 1m00s"
                        .to_string(),
                }
            );
        }

        #[test]
        fn a_pending_chunk_is_not_a_stall() {
            let report = describe(
                100 * MIB,
                8 * MIB,
                40 * MIB,
                40 * MIB,
                INTERVAL,
                Duration::from_secs(60),
            );
            assert_eq!(
                report,
                Report {
                    healthy: true,
                    message: "no 8.0 MiB chunk completed in the last 30s; 40.0 MiB of 100.0 MiB \
                              (40.0%) sent after 1m00s"
                        .to_string(),
                }
            );
        }

        #[test]
        fn zero_total_omits_percent_and_eta() {
            let report = describe(
                0,
                0,
                0,
                3 * MIB,
                Duration::from_secs(3),
                Duration::from_secs(3),
            );
            assert_eq!(
                report,
                Report {
                    healthy: true,
                    message: "3.0 MiB sent; 1.0 MiB/s over the last 3s, 1.0 MiB/s average"
                        .to_string(),
                }
            );
        }
    }

    mod intervals {
        use super::*;

        #[test]
        fn compares_each_reading_with_the_last() {
            let mut intervals = Intervals {
                total: 10 * MIB,
                unit: 0,
                interval: INTERVAL,
                prev: 0,
            };
            let reports = [
                intervals.report(3 * MIB, INTERVAL),
                intervals.report(3 * MIB, INTERVAL * 2),
            ];
            assert_eq!(
                reports,
                [
                    Report {
                        healthy: true,
                        message: "3.0 MiB of 10.0 MiB (30.0%) sent; 0.1 MiB/s over the last \
                                  30s, 0.1 MiB/s average; about 1m10s left"
                            .to_string(),
                    },
                    Report {
                        healthy: false,
                        message: "no progress in the last 30s; 3.0 MiB of 10.0 MiB (30.0%) \
                                  sent after 1m00s"
                            .to_string(),
                    },
                ]
            );
        }
    }

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
