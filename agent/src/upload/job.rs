// internal crates
use crate::filesys::File;

// external crates
use chrono::{DateTime, Utc};

#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    pub file: File,
    pub size: u64,
    pub digest: String,
    pub mtime: DateTime<Utc>,
    pub upload_rule_id: String,
    pub deployment_id: String,
    pub release_id: String,
}
