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
    /// When the agent first observed the file on the device. Mirrors the
    /// scanner's observation timestamps (`StableFile.first_observed_at` in
    /// `agent/src/scan/state.rs`) and is stamped into the backend upload
    /// mint request's `UploadSource.first_observed_at` (required field).
    pub first_observed_at: DateTime<Utc>,
    /// The most recent observation of the file before the upload was
    /// minted. Mirrors `StableFile.last_observed_at` in
    /// `agent/src/scan/state.rs` and is stamped into the backend upload
    /// mint request's `UploadSource.last_observed_at` (required field).
    pub last_observed_at: DateTime<Utc>,
    pub upload_rule_id: String,
    pub deployment_id: String,
}
