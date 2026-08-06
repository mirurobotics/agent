// internal crates
use crate::filesys::File;
use crate::models::DeletePolicy;

// external crates
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub file: File,
    pub size: u64,
    pub digest: String,
    pub mtime: DateTime<Utc>,
    pub first_observed_at: DateTime<Utc>,
    pub last_observed_at: DateTime<Utc>,
    pub upload_rule_id: String,
    pub deployment_id: String,
    pub delete_policy: DeletePolicy,
    pub delete_delay_secs: i64,
}
