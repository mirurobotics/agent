// standard crates
use std::collections::HashMap;

// internal crates
use crate::filesys::File;
use crate::models::FileRuleRetention;

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
    pub file_rule_id: String,
    pub deployment_id: String,
    #[serde(default)]
    pub retention: Option<FileRuleRetention>,
    /// The backend upload every attempt reuses, saved before the first
    /// transfer starts. `None` until `POST /uploads` succeeds.
    #[serde(default)]
    pub upload: Option<UploadCheckpoint>,
}

/// The backend upload (`upl_…`) a job reuses across attempts and restarts.
/// Holds everything the transfer needs besides credentials, because
/// `POST /uploads/{id}/credentials` only returns credentials.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UploadCheckpoint {
    pub id: String,
    pub bucket_name: String,
    pub object_key: String,
    pub metadata: HashMap<String, String>,
    /// The multipart upload started for this object, if any.
    pub multipart: Option<MultipartCheckpoint>,
}

/// A multipart upload started on S3 or GCS. `size` and `mtime` pin it to the
/// file version it was started for, and `part_size` to how the file was cut
/// into parts. If any no longer matches, the next attempt starts a new one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MultipartCheckpoint {
    pub id: String,
    pub size: u64,
    pub mtime: DateTime<Utc>,
    pub part_size: u64,
}
