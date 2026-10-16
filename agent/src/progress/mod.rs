// standard crates
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

// external crates
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior};
use tracing::{info, warn};

/// How often a [`Reporter`] logs a running transfer.
pub const REPORT_INTERVAL: Duration = Duration::from_secs(30);

/// Bytes moved by a long-running transfer of a fixed `total` size, shared
/// between the code that moves them and a [`Reporter`] that logs them. Counts
/// describe what the transfer code has handed off, not what the remote end has
/// acknowledged.
#[derive(Debug)]
pub struct Progress {
    total: u64,
    done: AtomicU64,
    rewinds: AtomicU64,
}

/// A point-in-time copy of a [`Progress`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub total: u64,
    pub done: u64,
    pub rewinds: u64,
}

impl Progress {
    pub fn new(total: u64) -> Self {
        Self {
            total,
            done: AtomicU64::new(0),
            rewinds: AtomicU64::new(0),
        }
    }

    pub fn add(&self, bytes: u64) {
        self.done.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Moves the position to `offset` and returns the previous position. A
    /// move backwards counts as a rewind: the transfer is resending data.
    pub fn seek_to(&self, offset: u64) -> u64 {
        let prev = self.done.swap(offset, Ordering::Relaxed);
        if offset < prev {
            self.rewinds.fetch_add(1, Ordering::Relaxed);
        }
        prev
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            total: self.total,
            done: self.done.load(Ordering::Relaxed),
            rewinds: self.rewinds.load(Ordering::Relaxed),
        }
    }
}

/// Logs a [`Progress`] every `interval` until it is finished or dropped. An
/// interval with no progress, or with a rewind, logs a warning, so a stalled or
/// retrying transfer shows up while it happens. Dropping an unfinished reporter
/// (the transfer was cancelled, e.g. by a deadline) logs how far it got.
pub struct Reporter {
    label: String,
    progress: Arc<Progress>,
    started: Instant,
    task: JoinHandle<()>,
    finished: bool,
}

impl Reporter {
    pub fn spawn(label: String, progress: Arc<Progress>, interval: Duration) -> Self {
        let started = Instant::now();
        let task = tokio::spawn(report_every(
            label.clone(),
            progress.clone(),
            started,
            interval,
        ));
        Self {
            label,
            progress,
            started,
            task,
            finished: false,
        }
    }

    /// Stops reporting and logs the final state of a transfer that returned.
    pub fn finish(mut self, succeeded: bool) {
        self.finished = true;
        self.log_final(if succeeded { "completed" } else { "failed" });
    }

    fn log_final(&self, outcome: &str) {
        let message = summary(outcome, self.progress.snapshot(), self.started.elapsed());
        if outcome == "completed" {
            info!("{}: {message}", self.label);
        } else {
            warn!("{}: {message}", self.label);
        }
    }
}

impl Drop for Reporter {
    fn drop(&mut self) {
        self.task.abort();
        if !self.finished {
            self.log_final("cancelled");
        }
    }
}

async fn report_every(
    label: String,
    progress: Arc<Progress>,
    started: Instant,
    interval: Duration,
) {
    let mut ticker = tokio::time::interval_at(started + interval, interval);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut prev = progress.snapshot();
    loop {
        ticker.tick().await;
        let now = progress.snapshot();
        let report = describe(prev, now, interval, started.elapsed());
        if report.healthy {
            info!("{label}: {}", report.message);
        } else {
            warn!("{label}: {}", report.message);
        }
        prev = now;
    }
}

/// One periodic progress line. `healthy` is false when the interval made no
/// progress or rewound.
#[derive(Debug, PartialEq, Eq)]
struct Report {
    healthy: bool,
    message: String,
}

fn describe(prev: Snapshot, now: Snapshot, interval: Duration, elapsed: Duration) -> Report {
    let position = position(now);
    let interval_text = duration(interval.as_secs());
    if now.rewinds > prev.rewinds {
        let rewinds = now.rewinds - prev.rewinds;
        return Report {
            healthy: false,
            message: format!(
                "rewound {rewinds} time(s) in the last {interval_text}; resending from {position}"
            ),
        };
    }
    if now.done <= prev.done {
        return Report {
            healthy: false,
            message: format!(
                "no progress in the last {interval_text}; {position} after {}",
                duration(elapsed.as_secs())
            ),
        };
    }
    let recent = rate(now.done - prev.done, interval);
    let average = rate(now.done, elapsed);
    let mut message = format!(
        "{position}; {}/s over the last {interval_text}, {}/s average",
        mib(recent),
        mib(average)
    );
    if now.total > now.done && average > 0 {
        let eta = (now.total - now.done) / average;
        message.push_str(&format!("; about {} left", duration(eta)));
    }
    Report {
        healthy: true,
        message,
    }
}

fn summary(outcome: &str, now: Snapshot, elapsed: Duration) -> String {
    let mut message = format!(
        "{outcome} after {}; {}, {}/s average",
        duration(elapsed.as_secs()),
        position(now),
        mib(rate(now.done, elapsed))
    );
    if now.rewinds > 0 {
        message.push_str(&format!("; rewound {} time(s)", now.rewinds));
    }
    message
}

/// `done of total (pct%) sent`, or just `done sent` when the total is unknown.
fn position(now: Snapshot) -> String {
    if now.total == 0 {
        return format!("{} sent", mib(now.done));
    }
    let pct = now.done as f64 * 100.0 / now.total as f64;
    format!("{} of {} ({pct:.1}%) sent", mib(now.done), mib(now.total))
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

    fn snap(total: u64, done: u64, rewinds: u64) -> Snapshot {
        Snapshot {
            total,
            done,
            rewinds,
        }
    }

    mod progress {
        use super::*;

        #[test]
        fn counts_bytes_and_rewinds() {
            let progress = Progress::new(10);
            progress.add(4);
            progress.add(3);
            assert_eq!(progress.seek_to(2), 7);
            assert_eq!(progress.snapshot(), snap(10, 2, 1));
        }

        #[test]
        fn seeking_forward_is_not_a_rewind() {
            let progress = Progress::new(0);
            assert_eq!(progress.seek_to(0), 0);
            assert_eq!(progress.seek_to(5), 0);
            assert_eq!(progress.snapshot(), snap(0, 5, 0));
        }
    }

    mod describe {
        use super::*;

        #[test]
        fn progressing_reports_rates_and_eta() {
            let report = describe(
                snap(100 * MIB, 10 * MIB, 0),
                snap(100 * MIB, 40 * MIB, 0),
                Duration::from_secs(30),
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
                snap(100 * MIB, 40 * MIB, 0),
                snap(100 * MIB, 40 * MIB, 0),
                Duration::from_secs(30),
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
        fn a_rewind_is_reported() {
            let report = describe(
                snap(100 * MIB, 40 * MIB, 0),
                snap(100 * MIB, 32 * MIB, 1),
                Duration::from_secs(30),
                Duration::from_secs(60),
            );
            assert_eq!(
                report,
                Report {
                    healthy: false,
                    message: "rewound 1 time(s) in the last 30s; resending from 32.0 MiB of \
                              100.0 MiB (32.0%) sent"
                        .to_string(),
                }
            );
        }

        #[test]
        fn unknown_total_omits_percent_and_eta() {
            let report = describe(
                snap(0, 0, 0),
                snap(0, 3 * MIB, 0),
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

    mod summary {
        use super::*;

        #[test]
        fn includes_rewinds_when_there_were_any() {
            assert_eq!(
                summary("failed", snap(4 * MIB, 2 * MIB, 2), Duration::from_secs(2)),
                "failed after 2s; 2.0 MiB of 4.0 MiB (50.0%) sent, 1.0 MiB/s average; \
                 rewound 2 time(s)"
            );
        }

        #[test]
        fn zero_elapsed_has_zero_rate() {
            assert_eq!(
                summary("completed", snap(0, 0, 0), Duration::ZERO),
                "completed after 0s; 0.0 MiB sent, 0.0 MiB/s average"
            );
        }
    }

    mod reporter {
        use super::*;

        // Runs one progressing and one stalled interval, then stops on finish.
        #[tokio::test(start_paused = true)]
        async fn finishing_stops_the_task() {
            let progress = Arc::new(Progress::new(10 * MIB));
            let reporter = Reporter::spawn(
                "test".to_string(),
                progress.clone(),
                Duration::from_secs(30),
            );
            let task = reporter.task.abort_handle();

            progress.add(MIB);
            tokio::time::sleep(Duration::from_secs(61)).await;
            assert!(!task.is_finished());

            reporter.finish(true);
            tokio::task::yield_now().await;

            assert!(task.is_finished());
        }

        #[tokio::test(start_paused = true)]
        async fn dropping_an_unfinished_reporter_stops_it() {
            let progress = Arc::new(Progress::new(0));
            let reporter = Reporter::spawn(
                "test".to_string(),
                progress.clone(),
                Duration::from_secs(30),
            );
            let task = reporter.task.abort_handle();

            drop(reporter);
            tokio::task::yield_now().await;

            assert!(task.is_finished());
        }
    }
}
