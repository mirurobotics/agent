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
    /// The backend upload this job reuses on every attempt, persisted before
    /// the first transfer starts. `None` until `POST /uploads` succeeds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload: Option<UploadRef>,
    /// Provider-side progress of the current transfer, persisted when a
    /// multipart upload starts so a later attempt can resume it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transfer: Option<TransferState>,
}

/// The backend upload (`upl_…`) a job reuses across attempts and restarts.
/// Holds everything the transfer needs besides credentials, because
/// `POST /uploads/{id}/credentials` only returns credentials.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UploadRef {
    pub id: String,
    pub bucket_id: String,
    pub bucket_name: String,
    pub object_key: String,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
    /// The object landed in the bucket; only the confirm is left. Lets a retry
    /// after a failed confirm skip the transfer, which also matters on GCS,
    /// where the downscoped token cannot overwrite an existing object.
    #[serde(default)]
    pub transferred: bool,
}

/// Provider-side progress of a transfer. One shape covers S3 and GCS, since
/// both go through the S3 multipart API.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TransferState {
    Multipart(MultipartState),
}

/// A started multipart upload. `size` and `mtime` pin it to the file version
/// it was started for; a file that no longer matches starts a fresh upload.
/// `part_size` is kept so a resume splits the file exactly as the first
/// attempt did, even if a newer agent picks part sizes differently.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MultipartState {
    pub multipart_upload_id: String,
    pub part_size: u64,
    pub size: u64,
    pub mtime: DateTime<Utc>,
}

/// A change to a job's upload state that must reach disk before the attempt
/// goes on. Produced by the executor, persisted by the uploader.
#[derive(Clone, Debug, PartialEq)]
pub enum Checkpoint {
    /// `POST /uploads` succeeded; later attempts reuse this upload.
    Created(UploadRef),
    /// A multipart upload started.
    TransferStarted(TransferState),
    /// The object landed; only the confirm is left.
    Transferred,
    /// The provider no longer has the transfer, or the file changed: the
    /// next transfer starts over on the same object key.
    TransferCleared,
    /// The backend no longer accepts the upload: the next attempt creates a
    /// new one.
    UploadCleared,
}

impl Job {
    /// Applies `checkpoint` to this job's persisted upload state.
    pub fn apply(&mut self, checkpoint: Checkpoint) {
        match checkpoint {
            Checkpoint::Created(upload) => {
                self.upload = Some(upload);
                self.transfer = None;
            }
            Checkpoint::TransferStarted(state) => self.transfer = Some(state),
            Checkpoint::Transferred => {
                if let Some(upload) = self.upload.as_mut() {
                    upload.transferred = true;
                }
                self.transfer = None;
            }
            Checkpoint::TransferCleared => self.transfer = None,
            Checkpoint::UploadCleared => {
                self.upload = None;
                self.transfer = None;
            }
        }
    }
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
            transfer: None,
        }
    }

    fn upload() -> UploadRef {
        UploadRef {
            id: "upl_1".to_string(),
            bucket_id: "bkt_1".to_string(),
            bucket_name: "my-bucket".to_string(),
            object_key: "logs/a.log".to_string(),
            metadata: HashMap::from([("device_id".to_string(), "dvc_1".to_string())]),
            transferred: false,
        }
    }

    fn multipart() -> TransferState {
        TransferState::Multipart(MultipartState {
            multipart_upload_id: "mpu_1".to_string(),
            part_size: 8 * 1024 * 1024,
            size: 42,
            mtime: DateTime::from_timestamp(900, 0).unwrap(),
        })
    }

    // A queue file written before resume support still loads, and a job
    // without upload state serializes exactly as before.
    #[test]
    fn job_without_upload_state_keeps_the_old_shape() {
        let value = serde_json::to_value(job()).unwrap();
        assert!(value.get("upload").is_none());
        assert!(value.get("transfer").is_none());
        let back: Job = serde_json::from_value(value).unwrap();
        assert_eq!(back, job());
    }

    #[test]
    fn upload_state_round_trips() {
        let mut job = job();
        job.upload = Some(upload());
        job.transfer = Some(multipart());
        let value = serde_json::to_value(&job).unwrap();
        assert_eq!(value["transfer"]["kind"], json!("multipart"));
        assert_eq!(value["transfer"]["multipart_upload_id"], json!("mpu_1"));
        let back: Job = serde_json::from_value(value).unwrap();
        assert_eq!(back, job);
    }

    #[test]
    fn apply_created_sets_the_upload_and_clears_the_transfer() {
        let mut job = job();
        job.transfer = Some(multipart());
        job.apply(Checkpoint::Created(upload()));
        assert_eq!(job.upload, Some(upload()));
        assert_eq!(job.transfer, None);
    }

    #[test]
    fn apply_transfer_started_and_cleared() {
        let mut job = job();
        job.upload = Some(upload());
        job.apply(Checkpoint::TransferStarted(multipart()));
        assert_eq!(job.transfer, Some(multipart()));
        job.apply(Checkpoint::TransferCleared);
        assert_eq!(job.transfer, None);
        assert_eq!(job.upload, Some(upload()));
    }

    #[test]
    fn apply_transferred_marks_the_upload_and_drops_the_transfer() {
        let mut job = job();
        job.upload = Some(upload());
        job.transfer = Some(multipart());
        job.apply(Checkpoint::Transferred);
        assert!(job.upload.as_ref().unwrap().transferred);
        assert_eq!(job.transfer, None);
    }

    #[test]
    fn apply_transferred_without_an_upload_is_a_no_op_on_the_upload() {
        let mut job = job();
        job.apply(Checkpoint::Transferred);
        assert_eq!(job.upload, None);
    }

    #[test]
    fn apply_upload_cleared_drops_everything() {
        let mut job = job();
        job.upload = Some(upload());
        job.transfer = Some(multipart());
        job.apply(Checkpoint::UploadCleared);
        assert_eq!(job.upload, None);
        assert_eq!(job.transfer, None);
    }
}
