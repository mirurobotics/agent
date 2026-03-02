// internal crates
use crate::deserialize_error;
use backend_api::models as backend_client;

// external crates
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

// ================================ GIT COMMIT ==================================== //
pub type GitCommitID = String;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GitCommit {
    pub id: String,
    pub sha: String,
    pub message: String,
    pub repository_owner: String,
    pub repository_name: String,
    pub repository_type: String,
    pub repository_url: String,
    pub commit_url: String,
    pub created_at: DateTime<Utc>,
}

impl Default for GitCommit {
    fn default() -> Self {
        Self {
            id: format!("unknown-{}", Uuid::new_v4()),
            sha: String::new(),
            message: String::new(),
            repository_owner: String::new(),
            repository_name: String::new(),
            repository_type: String::new(),
            repository_url: String::new(),
            commit_url: String::new(),
            created_at: DateTime::<Utc>::UNIX_EPOCH,
        }
    }
}

impl From<backend_client::GitCommit> for GitCommit {
    fn from(gc: backend_client::GitCommit) -> GitCommit {
        GitCommit {
            id: gc.id,
            sha: gc.sha,
            message: gc.message,
            repository_owner: gc.repository_owner,
            repository_name: gc.repository_name,
            repository_type: gc.repository_type.to_string(),
            repository_url: gc.repository_url,
            commit_url: gc.commit_url,
            created_at: gc
                .created_at
                .parse::<DateTime<Utc>>()
                .unwrap_or(DateTime::<Utc>::UNIX_EPOCH),
        }
    }
}

impl<'de> Deserialize<'de> for GitCommit {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        pub struct DeserializeGitCommit {
            id: String,
            sha: String,
            message: String,
            repository_owner: Option<String>,
            repository_name: Option<String>,
            repository_type: Option<String>,
            repository_url: Option<String>,
            commit_url: String,
            created_at: Option<DateTime<Utc>>,
        }

        let result = DeserializeGitCommit::deserialize(deserializer)?;
        let default = GitCommit::default();

        Ok(GitCommit {
            id: result.id,
            sha: result.sha,
            message: result.message,
            repository_owner: result.repository_owner.unwrap_or_else(|| {
                deserialize_error!("git_commit", "repository_owner", default.repository_owner)
            }),
            repository_name: result.repository_name.unwrap_or_else(|| {
                deserialize_error!("git_commit", "repository_name", default.repository_name)
            }),
            repository_type: result.repository_type.unwrap_or_else(|| {
                deserialize_error!("git_commit", "repository_type", default.repository_type)
            }),
            repository_url: result.repository_url.unwrap_or_else(|| {
                deserialize_error!("git_commit", "repository_url", default.repository_url)
            }),
            commit_url: result.commit_url,
            created_at: result.created_at.unwrap_or_else(|| {
                deserialize_error!("git_commit", "created_at", default.created_at)
            }),
        })
    }
}
