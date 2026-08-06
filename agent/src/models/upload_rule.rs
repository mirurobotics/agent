// internal crates
use crate::deserialize_error;
use crate::models::status::impl_status_enum;
use backend_api::models as backend_client;

// external crates
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde::Serialize;
use tracing::error;
use uuid::Uuid;

// =============================== DELETE POLICY =================================== //
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeletePolicy {
    #[default]
    Never,
    AfterUpload,
}

impl_status_enum!(
    enum DeletePolicy,
    default: Never,
    label: "delete policy",
    log: error,
    display: true,
    backend_type: backend_client::UploadDeletePolicy,
    unknown_backend: backend_client::UploadDeletePolicy::UploadDeletePolicyUnknown,
    mappings: [
        Never => "never" =>
            backend_client::UploadDeletePolicy::UPLOAD_DELETE_POLICY_NEVER,
        AfterUpload => "after_upload" =>
            backend_client::UploadDeletePolicy::UPLOAD_DELETE_POLICY_AFTER_UPLOAD,
    ]
);

// ============================ UPLOAD RULE SOURCE ================================= //
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UploadRuleSource {
    pub glob: String,
    pub stability_window_secs: i64,
}

impl From<backend_client::UploadRuleSource> for UploadRuleSource {
    fn from(source: backend_client::UploadRuleSource) -> UploadRuleSource {
        UploadRuleSource {
            glob: source.glob,
            stability_window_secs: source.stability_window_secs,
        }
    }
}

// ========================== UPLOAD RULE DESTINATION ============================= //
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UploadRuleDestination {
    pub bucket_id: String,
    pub bucket_name: String,
    pub path: String,
    pub delete_policy: DeletePolicy,
    /// Seconds to keep the file after it becomes deletable (i.e. after a
    /// confirmed upload under `AfterUpload`). Internal-only until openapi
    /// #212 lands: the backend cannot express it yet, so the wire mapping
    /// sets 0 (delete on the next sweep). `#[serde(default)]` keeps cached
    /// `upload_rules.json` written by older agents deserializable.
    #[serde(default)]
    pub delete_delay_secs: i64,
}

impl From<backend_client::UploadRuleDestination> for UploadRuleDestination {
    fn from(destination: backend_client::UploadRuleDestination) -> UploadRuleDestination {
        UploadRuleDestination {
            bucket_id: destination.bucket_id,
            bucket_name: destination.bucket_name,
            path: destination.path,
            delete_policy: (&destination.delete_policy).into(),
            // internal-only until openapi #212 adds it to the wire schema
            delete_delay_secs: 0,
        }
    }
}

// ================================ UPLOAD RULE =================================== //
pub type UploadRuleID = String;
pub type UploadCollectionID = String;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UploadRule {
    pub id: String,
    pub upload_collection_id: String,
    pub upload_collection_name: String,
    pub digest: String,
    pub source: UploadRuleSource,
    pub destination: UploadRuleDestination,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Default for UploadRule {
    fn default() -> Self {
        Self {
            id: format!("unknown-{}", Uuid::new_v4()),
            upload_collection_id: format!("unknown-{}", Uuid::new_v4()),
            upload_collection_name: String::new(),
            digest: String::new(),
            source: UploadRuleSource::default(),
            destination: UploadRuleDestination::default(),
            created_at: DateTime::<Utc>::UNIX_EPOCH,
            updated_at: DateTime::<Utc>::UNIX_EPOCH,
        }
    }
}

impl From<backend_client::BaseUploadRule> for UploadRule {
    fn from(rule: backend_client::BaseUploadRule) -> UploadRule {
        UploadRule {
            id: rule.id,
            upload_collection_id: rule.upload_collection_id,
            upload_collection_name: rule.upload_collection_name,
            digest: rule.digest,
            source: (*rule.source).into(),
            destination: (*rule.destination).into(),
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

impl<'de> Deserialize<'de> for UploadRule {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        pub struct DeserializeUploadRule {
            id: String,
            upload_collection_id: String,
            upload_collection_name: String,
            digest: String,
            source: UploadRuleSource,
            destination: UploadRuleDestination,
            created_at: Option<DateTime<Utc>>,
            updated_at: Option<DateTime<Utc>>,
        }

        let result = DeserializeUploadRule::deserialize(deserializer)?;
        let default = UploadRule::default();

        Ok(UploadRule {
            id: result.id,
            upload_collection_id: result.upload_collection_id,
            upload_collection_name: result.upload_collection_name,
            digest: result.digest,
            source: result.source,
            destination: result.destination,
            created_at: result.created_at.unwrap_or_else(|| {
                deserialize_error!("upload_rule", "created_at", default.created_at)
            }),
            updated_at: result.updated_at.unwrap_or_else(|| {
                deserialize_error!("upload_rule", "updated_at", default.updated_at)
            }),
        })
    }
}
