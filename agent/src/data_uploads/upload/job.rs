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
    #[serde(default)]
    pub upload: Option<UploadCheckpoint>,
}

/// Holds the destination and metadata too, since
/// `POST /uploads/{id}/credentials` returns only credentials.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UploadCheckpoint {
    pub miru_upload_id: String,
    pub bucket_name: String,
    pub object_key: String,
    pub metadata: HashMap<String, String>,
    pub multipart: Option<MultipartCheckpoint>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MultipartCheckpoint {
    pub multipart_upload_id: String,
    pub size: u64,
    pub mtime: DateTime<Utc>,
    pub part_size: u64,
}
