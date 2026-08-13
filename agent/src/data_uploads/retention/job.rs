// internal crates
use crate::filesys::File;

// external crates
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub file: File,
    pub size: u64,
    pub digest: String,
    pub mtime: DateTime<Utc>,
    pub first_observed_at: DateTime<Utc>,
    pub last_observed_at: DateTime<Utc>,
    pub ttl_secs: u64,
    pub file_rule_id: String,
    pub deployment_id: String,
}

impl Job {
    /// `last_observed_at` plus the TTL. A TTL too large to represent saturates
    /// to the far future — the job is simply never due.
    pub fn due_at(&self) -> DateTime<Utc> {
        i64::try_from(self.ttl_secs)
            .ok()
            .and_then(Duration::try_seconds)
            .and_then(|ttl| self.last_observed_at.checked_add_signed(ttl))
            .unwrap_or(DateTime::<Utc>::MAX_UTC)
    }
}

#[cfg(test)]
mod tests {
    // internal crates
    use super::Job;
    use crate::filesys::File;

    // external crates
    use chrono::{DateTime, Utc};

    fn job(observed_secs: i64, ttl_secs: u64) -> Job {
        let observed_at = DateTime::from_timestamp(observed_secs, 0).unwrap();
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
        }
    }

    #[test]
    fn due_at_adds_ttl_and_saturates_on_overflow() {
        assert_eq!(
            job(1000, 300).due_at(),
            DateTime::from_timestamp(1300, 0).unwrap()
        );

        // a TTL beyond what chrono can represent saturates to "never due"
        // instead of panicking.
        assert_eq!(job(1000, u64::MAX).due_at(), DateTime::<Utc>::MAX_UTC);
    }
}
