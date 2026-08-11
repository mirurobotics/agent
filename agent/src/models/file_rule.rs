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
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FileRuleSource {
    pub glob: String,
    pub stability_window_secs: i64,
}

impl From<backend_client::UploadRuleSource> for FileRuleSource {
    fn from(source: backend_client::UploadRuleSource) -> FileRuleSource {
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

// ============================ FILE RULE RETENTION ================================ //
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRuleRetention {
    pub require_upload: bool,
    pub ttl_secs: u64,
}

// ================================= FILE RULE ===================================== //
pub type FileRuleID = String;
pub type UploadCollectionID = String;

#[derive(Clone, Debug, PartialEq, Serialize)]
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

impl From<backend_client::BaseUploadRule> for FileRule {
    fn from(rule: backend_client::BaseUploadRule) -> FileRule {
        let destination = *rule.destination;
        let retention = match destination.delete_policy {
            backend_client::UploadDeletePolicy::UPLOAD_DELETE_POLICY_AFTER_UPLOAD => {
                Some(FileRuleRetention {
                    require_upload: true,
                    ttl_secs: 0,
                })
            }
            backend_client::UploadDeletePolicy::UPLOAD_DELETE_POLICY_NEVER
            | backend_client::UploadDeletePolicy::UploadDeletePolicyUnknown => None,
        };
        FileRule {
            id: rule.id,
            name: rule.upload_collection_name.clone(),
            digest: rule.digest,
            source: (*rule.source).into(),
            upload: Some(FileRuleUpload {
                upload_collection_id: rule.upload_collection_id,
                upload_collection_name: rule.upload_collection_name,
                bucket_id: destination.bucket_id,
                bucket_name: destination.bucket_name,
                path: destination.path,
            }),
            retention,
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
