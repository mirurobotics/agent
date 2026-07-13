// standard crates
use std::path::PathBuf;

// internal crates
use crate::filesys::{File, PathExt};

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

/// The identity of an upload job: two jobs with the same key are the same
/// upload. Used for dedup against queued and in-flight jobs.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DedupKey {
    pub upload_rule_id: String,
    pub path: PathBuf,
    pub digest: String,
}

impl Job {
    pub fn dedup_key(&self) -> DedupKey {
        DedupKey {
            upload_rule_id: self.upload_rule_id.clone(),
            path: self.file.path().clone(),
            digest: self.digest.clone(),
        }
    }
}
