// standard crates
use std::time::Duration;

// internal crates
use crate::progress::format::{duration, mib, position, rate};

/// Turns one progress reading per interval into a report line, comparing each
/// reading with the one before it.
pub(super) struct Intervals {
    total: u64,
    unit: u64,
    interval: Duration,
    prev: u64,
}

impl Intervals {
    pub(super) fn new(total: u64, unit: u64, interval: Duration) -> Self {
        Self {
            total,
            unit,
            interval,
            prev: 0,
        }
    }

    pub(super) fn report(&mut self, now: u64, elapsed: Duration) -> Report {
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
pub(super) struct Report {
    pub(super) healthy: bool,
    pub(super) message: String,
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

#[cfg(test)]
mod tests {
    // internal crates
    use super::*;

    const MIB: u64 = 1024 * 1024;
    const INTERVAL: Duration = Duration::from_secs(30);

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
            let mut intervals = Intervals::new(10 * MIB, 0, INTERVAL);
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
}
