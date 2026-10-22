// standard crates
use std::time::Duration;

// internal crates
use crate::clock::uptime;
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
    /// The robot's system clock when the file was last observed, for logs.
    /// The TTL is counted by the two deadlines below, which never read the
    /// system clock.
    pub last_observed_at: DateTime<Utc>,
    pub ttl_secs: u64,
    pub file_rule_id: String,
    pub deployment_id: String,
    /// When the TTL runs out on the agent's uptime clock
    /// ([`crate::clock::Uptime`]), in seconds. The deleter sets it on
    /// enqueue. `None` only for a job saved before this field existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uptime_due_secs: Option<u64>,
    /// When the TTL runs out in server time, if the agent knew server time
    /// when the job was enqueued. The deleter sets it on enqueue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_due_at: Option<DateTime<Utc>>,
}

impl Job {
    /// The uptime, in whole seconds, at which the TTL runs out counting from
    /// `uptime_now`. Resolution is one second, like the TTL itself.
    pub fn uptime_due_from(&self, uptime_now: Duration) -> u64 {
        uptime_now.as_secs().saturating_add(self.ttl_secs)
    }

    /// The server time at which the TTL runs out, counting from `server_now`.
    pub fn server_due_from(&self, server_now: DateTime<Utc>) -> DateTime<Utc> {
        i64::try_from(self.ttl_secs)
            .ok()
            .and_then(TimeDelta::try_seconds)
            .and_then(|ttl| server_now.checked_add_signed(ttl))
            .unwrap_or(DateTime::<Utc>::MAX_UTC)
    }

    /// The uptime deadline as the queue compares it (see
    /// [`uptime::as_datetime`]). A job without one is never ready.
    pub fn ready_at(&self) -> DateTime<Utc> {
        self.uptime_due_secs
            .map(|secs| uptime::as_datetime(Duration::from_secs(secs)))
            .unwrap_or(DateTime::<Utc>::MAX_UTC)
    }
}

#[cfg(test)]
mod tests {
    // standard crates
    use std::time::Duration;

    // internal crates
    use super::Job;
    use crate::filesys::File;

    // external crates
    use chrono::{DateTime, Utc};

    fn job(ttl_secs: u64) -> Job {
        let observed_at = DateTime::from_timestamp(1000, 0).unwrap();
        Job {
            file: File::new("/data/a.log"),
            size: 4,
            digest: "sha256:unused".to_string(),
            mtime: DateTime::from_timestamp(900, 0).unwrap(),
            first_observed_at: observed_at,
            last_observed_at: observed_at,
            ttl_secs,
            file_rule_id: "file_rule_1".to_string(),
            deployment_id: "dpl_1".to_string(),
            uptime_due_secs: None,
            server_due_at: None,
        }
    }

    #[test]
    fn uptime_due_adds_ttl_and_saturates() {
        assert_eq!(job(300).uptime_due_from(Duration::from_secs(100)), 400);
        assert_eq!(
            job(300).uptime_due_from(Duration::from_millis(100_500)),
            400
        );
        assert_eq!(
            job(u64::MAX).uptime_due_from(Duration::from_secs(1)),
            u64::MAX
        );
    }

    #[test]
    fn server_due_adds_ttl_and_saturates() {
        let now = DateTime::from_timestamp(1000, 0).unwrap();
        assert_eq!(
            job(300).server_due_from(now),
            DateTime::from_timestamp(1300, 0).unwrap()
        );
        assert_eq!(job(u64::MAX).server_due_from(now), DateTime::<Utc>::MAX_UTC);
    }

    #[test]
    fn ready_at_is_the_uptime_deadline() {
        assert_eq!(job(300).ready_at(), DateTime::<Utc>::MAX_UTC);
        let due = Job {
            uptime_due_secs: Some(400),
            ..job(300)
        };
        assert_eq!(due.ready_at(), DateTime::from_timestamp(400, 0).unwrap());
    }
}
