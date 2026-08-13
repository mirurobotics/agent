// internal crates
use crate::deserialize_error;
use backend_api::models as backend_client;

// external crates
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde::Serialize;
use tracing::error;
use uuid::Uuid;

// ============================= FILE RULE SOURCE ================================== //
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRuleSource {
    pub glob: String,
    pub stability_window_secs: i64,
}

impl From<backend_client::FileRuleSource> for FileRuleSource {
    fn from(source: backend_client::FileRuleSource) -> FileRuleSource {
        FileRuleSource {
            glob: source.glob,
            stability_window_secs: source.stability_window_secs,
        }
    }
}

// ============================= FILE RULE UPLOAD ================================== //
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRuleUpload {
    pub upload_collection_id: UploadCollectionID,
    pub upload_collection_name: String,
    pub bucket_id: String,
    pub bucket_name: String,
    pub path: String,
}

impl From<backend_client::FileRuleUpload> for FileRuleUpload {
    fn from(upload: backend_client::FileRuleUpload) -> FileRuleUpload {
        FileRuleUpload {
            upload_collection_id: upload.upload_collection_id,
            upload_collection_name: upload.upload_collection_name,
            bucket_id: upload.bucket_id,
            bucket_name: upload.bucket_name,
            path: upload.path,
        }
    }
}

// ============================ FILE RULE RETENTION ================================ //
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRuleRetention {
    pub require_upload: bool,
    pub ttl_secs: u64,
}

// ================================= FILE RULE ===================================== //
pub type FileRuleID = String;
pub type UploadCollectionID = String;

#[derive(Clone, Debug, PartialEq, Serialize, Eq)]
pub struct FileRule {
    pub id: FileRuleID,
    pub name: String,
    pub digest: String,
    pub source: FileRuleSource,
    pub upload: Option<FileRuleUpload>,
    pub retention: Option<FileRuleRetention>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Default for FileRule {
    fn default() -> Self {
        Self {
            id: format!("unknown-{}", Uuid::new_v4()),
            name: String::new(),
            digest: String::new(),
            source: FileRuleSource::default(),
            upload: None,
            retention: None,
            created_at: DateTime::<Utc>::UNIX_EPOCH,
            updated_at: DateTime::<Utc>::UNIX_EPOCH,
        }
    }
}

impl From<backend_client::BaseFileRule> for FileRule {
    fn from(rule: backend_client::BaseFileRule) -> FileRule {
        FileRule {
            id: rule.id,
            name: rule.name,
            digest: rule.digest,
            source: (*rule.source).into(),
            upload: rule.upload.map(|u| (*u).into()),
            retention: rule.retention.map(|r| FileRuleRetention {
                require_upload: r.require_upload.unwrap_or(false),
                ttl_secs: r.ttl_secs.max(0) as u64,
            }),
            created_at: rule
                .created_at
                .parse::<DateTime<Utc>>()
                .unwrap_or_else(|e| {
                    error!("Error parsing created_at: {}", e);
                    DateTime::<Utc>::UNIX_EPOCH
                }),
            updated_at: rule
                .updated_at
                .parse::<DateTime<Utc>>()
                .unwrap_or_else(|e| {
                    error!("Error parsing updated_at: {}", e);
                    DateTime::<Utc>::UNIX_EPOCH
                }),
        }
    }
}

impl<'de> Deserialize<'de> for FileRule {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        pub struct DeserializeFileRule {
            id: String,
            name: String,
            digest: String,
            source: FileRuleSource,
            #[serde(default)]
            upload: Option<FileRuleUpload>,
            #[serde(default)]
            retention: Option<FileRuleRetention>,
            created_at: Option<DateTime<Utc>>,
            updated_at: Option<DateTime<Utc>>,
        }

        let result = DeserializeFileRule::deserialize(deserializer)?;
        let default = FileRule::default();

        Ok(FileRule {
            id: result.id,
            name: result.name,
            digest: result.digest,
            source: result.source,
            upload: result.upload,
            retention: result.retention,
            created_at: result.created_at.unwrap_or_else(|| {
                deserialize_error!("file_rule", "created_at", default.created_at)
            }),
            updated_at: result.updated_at.unwrap_or_else(|| {
                deserialize_error!("file_rule", "updated_at", default.updated_at)
            }),
        })
    }
}
