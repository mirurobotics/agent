// internal crates
use crate::deserialize_error;
use backend_api::models as backend_client;

// external crates
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde::Serialize;
use tracing::error;
use uuid::Uuid;

// =============================== DELETE POLICY =================================== //
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeletePolicy {
    #[default]
    Never,
    AfterUpload,
}

impl std::fmt::Display for DeletePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            DeletePolicy::Never => write!(f, "never"),
            DeletePolicy::AfterUpload => write!(f, "after_upload"),
        }
    }
}

impl From<&backend_client::UploadDeletePolicy> for DeletePolicy {
    fn from(policy: &backend_client::UploadDeletePolicy) -> DeletePolicy {
        match policy {
            backend_client::UploadDeletePolicy::UPLOAD_DELETE_POLICY_NEVER => DeletePolicy::Never,
            backend_client::UploadDeletePolicy::UPLOAD_DELETE_POLICY_AFTER_UPLOAD => {
                DeletePolicy::AfterUpload
            }
            other => {
                let default = DeletePolicy::Never;
                error!(
                    "upload delete policy backend value {:?} is not recognized, defaulting to {:?}",
                    other, default
                );
                default
            }
        }
    }
}

impl<'de> Deserialize<'de> for DeletePolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = <String as serde::Deserialize>::deserialize(deserializer)?;
        let default = DeletePolicy::default();
        match s.as_str() {
            "never" => Ok(DeletePolicy::Never),
            "after_upload" => Ok(DeletePolicy::AfterUpload),
            status => {
                error!(
                    "delete policy '{}' is not valid, defaulting to {:?}",
                    status, default
                );
                Ok(default)
            }
        }
    }
}

// ============================ UPLOAD RULE SOURCE ================================= //
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UploadRuleSource {
    pub glob: String,
    pub poll_interval: String,
    pub stability_window: String,
}

impl From<backend_client::UploadRuleSource> for UploadRuleSource {
    fn from(source: backend_client::UploadRuleSource) -> UploadRuleSource {
        UploadRuleSource {
            glob: source.glob,
            poll_interval: source.poll_interval,
            stability_window: source.stability_window,
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
}

impl From<backend_client::UploadRuleDestination> for UploadRuleDestination {
    fn from(destination: backend_client::UploadRuleDestination) -> UploadRuleDestination {
        UploadRuleDestination {
            bucket_id: destination.bucket_id,
            bucket_name: destination.bucket_name,
            path: destination.path,
            delete_policy: (&destination.delete_policy).into(),
        }
    }
}

// ================================ UPLOAD RULE =================================== //
pub type UploadRuleID = String;

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
