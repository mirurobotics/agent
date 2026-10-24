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
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default)]
    pub metadata: HashMap<String, String>,
    /// The multipart upload in progress for this object, if one started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transfer: Option<TransferState>,
}

/// A started multipart upload, on S3 or GCS. `size`, `mtime` and `part_size`
/// pin it to the file version and part layout it was started with; if any
/// of them no longer match, the next attempt starts a new one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransferState {
    pub multipart_upload_id: String,
    pub part_size: u64,
    pub size: u64,
    pub mtime: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    // internal crates
    use super::*;

    // external crates
    use serde_json::json;

    fn job() -> Job {
        Job {
            file: File::new("/data/a.log"),
            size: 42,
            digest: "sha256:a".to_string(),
            mtime: DateTime::from_timestamp(900, 0).unwrap(),
            first_observed_at: DateTime::from_timestamp(1000, 0).unwrap(),
            last_observed_at: DateTime::from_timestamp(1000, 0).unwrap(),
            file_rule_id: "rule_1".to_string(),
            deployment_id: "dpl_1".to_string(),
            retention: None,
            upload: None,
        }
    }

    // A queue file written before resume support still loads, and a job
    // without an upload serializes exactly as before.
    #[test]
    fn job_without_an_upload_keeps_the_old_shape() {
        let value = serde_json::to_value(job()).unwrap();
        assert!(value.get("upload").is_none());
        let back: Job = serde_json::from_value(value).unwrap();
        assert_eq!(back, job());
    }

    #[test]
    fn upload_round_trips() {
        let mut job = job();
        job.upload = Some(UploadRef {
            id: "upl_1".to_string(),
            bucket_name: "my-bucket".to_string(),
            object_key: "logs/a.log".to_string(),
            metadata: HashMap::from([("device_id".to_string(), "dvc_1".to_string())]),
            transfer: Some(TransferState {
                multipart_upload_id: "mpu_1".to_string(),
                part_size: 8 * 1024 * 1024,
                size: 42,
                mtime: DateTime::from_timestamp(900, 0).unwrap(),
            }),
        });
        let value = serde_json::to_value(&job).unwrap();
        assert_eq!(
            value["upload"]["transfer"]["multipart_upload_id"],
            json!("mpu_1")
        );
        let back: Job = serde_json::from_value(value).unwrap();
        assert_eq!(back, job);
    }
}
