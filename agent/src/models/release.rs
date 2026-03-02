// internal crates
use crate::deserialize_error;
use backend_api::models as backend_client;

// external crates
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

// ================================ RELEASE ========================================= //
pub type ReleaseID = String;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Release {
    pub id: String,
    pub version: String,
    pub git_commit_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Default for Release {
    fn default() -> Self {
        Self {
            id: format!("unknown-{}", Uuid::new_v4()),
            version: String::new(),
            git_commit_id: None,
            created_at: DateTime::<Utc>::UNIX_EPOCH,
            updated_at: DateTime::<Utc>::UNIX_EPOCH,
        }
    }
}

impl From<backend_client::Release> for Release {
    fn from(release: backend_client::Release) -> Release {
        Release {
            id: release.id,
            version: release.version,
            git_commit_id: release.git_commit_id,
            created_at: release
                .created_at
                .parse::<DateTime<Utc>>()
                .unwrap_or(DateTime::<Utc>::UNIX_EPOCH),
            updated_at: release
                .updated_at
                .parse::<DateTime<Utc>>()
                .unwrap_or(DateTime::<Utc>::UNIX_EPOCH),
        }
    }
}

impl<'de> Deserialize<'de> for Release {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        pub struct DeserializeRelease {
            id: String,
            version: String,
            git_commit_id: Option<String>,
            created_at: Option<DateTime<Utc>>,
            updated_at: Option<DateTime<Utc>>,
        }

        let result = DeserializeRelease::deserialize(deserializer)?;
        let default = Release::default();

        Ok(Release {
            id: result.id,
            version: result.version,
            git_commit_id: result.git_commit_id,
            created_at: result
                .created_at
                .unwrap_or_else(|| deserialize_error!("release", "created_at", default.created_at)),
            updated_at: result
                .updated_at
                .unwrap_or_else(|| deserialize_error!("release", "updated_at", default.updated_at)),
        })
    }
}
