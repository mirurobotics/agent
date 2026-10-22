// standard crates
use std::time::Duration;

// internal crates
use crate::filesys::File;

// external crates
use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub file: File,
    pub size: u64,
    pub digest: String,
    pub mtime: DateTime<Utc>,
    pub first_observed_at: DateTime<Utc>,
    /// The robot's system clock when the file was last observed. Shown in
    /// logs only: the TTL is counted from [`Job::expiry`], which never reads
    /// the system clock.
    pub last_observed_at: DateTime<Utc>,
    pub ttl_secs: u64,
    pub file_rule_id: String,
    pub deployment_id: String,
    /// When the TTL runs out. The deleter stamps it on enqueue and keeps it
    /// current, so producers leave it at its default.
    #[serde(default, skip_serializing_if = "Expiry::is_unset")]
    pub expiry: Expiry,
}

impl Job {
    /// The TTL as a duration.
    pub fn ttl(&self) -> Duration {
        Duration::from_secs(self.ttl_secs)
    }
}

/// When a delete job's TTL runs out, in terms that do not depend on the
/// robot's system clock being right.
///
/// The deadline is in server time when the agent knows it. The TTL left is
/// also saved, counted on the monotonic clock, so a restart that has not
/// heard from the server yet can carry on counting. Time the agent is not
/// running never counts toward the TTL, so a file is kept longer, never
/// shorter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Expiry {
    /// Server time at which the TTL runs out. `None` until the agent has
    /// known the server's time while the job was queued.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_expires_at: Option<DateTime<Utc>>,
    /// TTL left, in seconds, as of the latest checkpoint. `None` means the
    /// whole TTL: a job saved before expiries existed, whose stamp came from
    /// the system clock, starts its TTL over.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining_ttl_secs: Option<u64>,
}

impl Expiry {
    /// The expiry of a job whose TTL starts now: the whole TTL is left, and
    /// the deadline is in server time if the server's time is known.
    pub fn start(ttl_secs: u64, server_now: Option<DateTime<Utc>>) -> Self {
        Self {
            server_expires_at: server_now
                .map(|now| add_duration(now, Duration::from_secs(ttl_secs))),
            remaining_ttl_secs: Some(ttl_secs),
        }
    }

    /// The TTL left at the latest checkpoint, or the whole `ttl` if none was
    /// saved.
    pub fn saved_remaining(&self, ttl: Duration) -> Duration {
        self.remaining_ttl_secs
            .map(Duration::from_secs)
            .unwrap_or(ttl)
    }

    fn is_unset(&self) -> bool {
        *self == Self::default()
    }
}

/// `at + duration`, saturating to the far future instead of overflowing.
pub(crate) fn add_duration(at: DateTime<Utc>, duration: Duration) -> DateTime<Utc> {
    TimeDelta::from_std(duration)
        .ok()
        .and_then(|delta| at.checked_add_signed(delta))
        .unwrap_or(DateTime::<Utc>::MAX_UTC)
}

#[cfg(test)]
mod tests {
    // standard crates
    use std::time::Duration;

    // internal crates
    use super::{add_duration, Expiry};

    // external crates
    use chrono::{DateTime, Utc};

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).unwrap()
    }

    #[test]
    fn start_sets_the_server_deadline_only_when_server_time_is_known() {
        assert_eq!(
            Expiry::start(300, Some(at(1000))),
            Expiry {
                server_expires_at: Some(at(1300)),
                remaining_ttl_secs: Some(300),
            }
        );
        assert_eq!(
            Expiry::start(300, None),
            Expiry {
                server_expires_at: None,
                remaining_ttl_secs: Some(300),
            }
        );
    }

    #[test]
    fn saved_remaining_falls_back_to_the_whole_ttl() {
        let ttl = Duration::from_secs(300);
        assert_eq!(Expiry::default().saved_remaining(ttl), ttl);
        let saved = Expiry {
            remaining_ttl_secs: Some(10),
            ..Expiry::default()
        };
        assert_eq!(saved.saved_remaining(ttl), Duration::from_secs(10));
    }

    #[test]
    fn add_duration_saturates_on_overflow() {
        assert_eq!(add_duration(at(1000), Duration::from_secs(300)), at(1300));
        // a TTL beyond what chrono can represent saturates to "never due"
        // instead of panicking.
        assert_eq!(
            add_duration(at(1000), Duration::from_secs(u64::MAX)),
            DateTime::<Utc>::MAX_UTC
        );
    }
}
