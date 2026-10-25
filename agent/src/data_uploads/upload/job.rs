// internal crates
use crate::filesys::File;
use crate::models::FileRuleRetention;

// external crates
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub file: File,
    pub size: u64,
    pub digest: String,
    pub mtime: DateTime<Utc>,
    pub first_observed_at: DateTime<Utc>,
    pub last_observed_at: DateTime<Utc>,
    pub file_rule_id: String,
    pub deployment_id: String,
    #[serde(default)]
    pub retention: Option<FileRuleRetention>,
    /// Minted with the job and sent with every attempt to create its upload,
    /// so Miru keeps one upload for the job across retries and restarts.
    #[serde(default = "Uuid::new_v4")]
    pub idempotency_key: Uuid,
}
