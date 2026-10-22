// internal crates
use crate::cooldown;
use crate::errors::Error;

// external crates
use chrono::{DateTime, TimeDelta, Utc};

/// When a queued job that failed is retried, and when it is given up on. A
/// network error doesn't count as an attempt: the job waits a flat
/// `backoff.base_secs`. Any other failure counts, and the job waits out a
/// backoff that grows with its attempts. Whatever the error, a job older than
/// `max_age` is dropped.
#[derive(Clone, Copy, Debug)]
pub struct RetryPolicy {
    /// Total counted attempts per job before it is dropped.
    pub attempts: u32,
    /// Backoff between counted attempts; the exponent is the job's attempt
    /// count minus one.
    pub backoff: cooldown::Backoff,
    /// Age past which a failing job is dropped, whatever the error. This is
    /// what ends a job that only ever fails with network errors.
    pub max_age: TimeDelta,
}

impl Default for RetryPolicy {
    /// 30 attempts, 10 s doubling to 1 h, and a 7-day backstop.
    fn default() -> Self {
        Self {
            attempts: 30,
            backoff: cooldown::Backoff {
                base_secs: 10,
                growth_factor: 2,
                max_secs: 3600,
            },
            max_age: TimeDelta::days(7),
        }
    }
}

/// What to do with a job whose attempt failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Retry it in `wait_secs`, recording `attempts`.
    Retry { attempts: u32, wait_secs: i64 },
    /// Drop it: the error says it can never succeed.
    Terminal { attempts: u32 },
    /// Drop it: it has used up its attempts.
    OutOfAttempts { attempts: u32 },
    /// Drop it: it is older than `max_age`.
    TooOld { attempts: u32, age: TimeDelta },
}

impl RetryPolicy {
    /// The latest deadline a retry can be given from `now`. A later one came
    /// from a clock jump and should be pulled back to it.
    pub fn horizon(&self, now: DateTime<Utc>) -> DateTime<Utc> {
        now + TimeDelta::seconds(self.backoff.max_secs.max(0))
    }

    /// The verdict on a job that failed with `err` after `attempts` counted
    /// attempts, `age` after it was first seen.
    pub fn on_failure(&self, err: &impl Error, attempts: u32, age: TimeDelta) -> Verdict {
        let network = err.is_network_conn_err();
        let attempts = if network {
            attempts
        } else {
            attempts.saturating_add(1)
        };
        if !network && err.is_terminal() {
            return Verdict::Terminal { attempts };
        }
        if age >= self.max_age {
            return Verdict::TooOld { attempts, age };
        }
        if network {
            let wait_secs = self.backoff.base_secs.max(0);
            return Verdict::Retry {
                attempts,
                wait_secs,
            };
        }
        if attempts >= self.attempts {
            return Verdict::OutOfAttempts { attempts };
        }
        let wait_secs = cooldown::calc(&self.backoff, attempts - 1).max(0);
        Verdict::Retry {
            attempts,
            wait_secs,
        }
    }
}
