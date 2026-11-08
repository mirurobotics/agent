// standard crates
use std::collections::HashMap;

// internal crates
use crate::filesys::File;
use crate::models::FileRuleRetention;
use crate::s3;

// external crates
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub file: File,
    pub size: u64,
    pub digest: String,
    /// The scan-time CRC32C that the object store checks the upload against.
    /// `None` for jobs queued before uploads carried a checksum.
    #[serde(default)]
    pub crc32c: Option<u32>,
    pub mtime: DateTime<Utc>,
    pub first_observed_at: DateTime<Utc>,
    pub last_observed_at: DateTime<Utc>,
    pub file_rule_id: String,
    pub deployment_id: String,
    #[serde(default)]
    pub retention: Option<FileRuleRetention>,
    #[serde(default)]
    pub checkpoint: Option<UploadCheckpoint>,
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

impl UploadCheckpoint {
    /// The upload's physical bucket and object key.
    pub(crate) fn object(&self) -> s3::Object {
        s3::Object {
            bucket: self.bucket_name.clone(),
            key: self.object_key.clone(),
        }
    }

    /// The saved multipart upload, if it was started for a file of `src`'s size
    /// with the part size this agent picks.
    pub(crate) fn resumable_multipart(&self, src: &s3::Source) -> Option<&MultipartCheckpoint> {
        let multipart = self.multipart.as_ref()?;
        let part_size = s3::Store::part_size_for(src.size);
        let resumable = src.size == multipart.size && part_size == multipart.part_size;
        resumable.then_some(multipart)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MultipartCheckpoint {
    pub multipart_upload_id: String,
    pub size: u64,
    pub part_size: u64,
}
