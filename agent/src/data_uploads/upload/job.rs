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
    pub upload: Option<UploadRef>,
}

/// The backend upload (`upl_…`) a job reuses across attempts and restarts.
/// Holds everything the transfer needs besides credentials, because
/// `POST /uploads/{id}/credentials` only returns credentials.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UploadRef {
    pub id: String,
    pub bucket_name: String,
    pub object_key: String,
    pub metadata: HashMap<String, String>,
    /// The multipart upload started for this object, if any.
    pub multipart: Option<MultipartRef>,
}

/// A multipart upload started on S3 or GCS. `size` and `mtime` pin it to the
/// file version it was started for, and `part_size` to how the file was cut
/// into parts. If any no longer matches, the next attempt starts a new one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MultipartRef {
    pub id: String,
    pub size: u64,
    pub mtime: DateTime<Utc>,
    pub part_size: u64,
}

#[cfg(test)]
mod tests {
    // internal crates
    use super::*;
    use crate::test_utils::upload::{make_job, upload_ref};

    // external crates
    use serde_json::json;

    #[test]
    fn upload_round_trips() {
        let mut job = make_job("a.log");
        job.upload = Some(UploadRef {
            multipart: Some(MultipartRef {
                id: "mpu_1".to_string(),
                size: 42,
                mtime: DateTime::from_timestamp(900, 0).unwrap(),
                part_size: 32,
            }),
            ..upload_ref()
        });
        let value = serde_json::to_value(&job).unwrap();
        assert_eq!(value["upload"]["multipart"]["id"], json!("mpu_1"));
        let back: Job = serde_json::from_value(value).unwrap();
        assert_eq!(back, job);
    }
}
