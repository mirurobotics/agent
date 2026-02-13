// internal crates
use crate::deserialize_error;

// external crates
use chrono::{DateTime, TimeDelta, Utc};
use serde::Deserialize;
use serde::Serialize;
use tracing::warn;
use uuid::Uuid;

// =========================== DEPLOYMENT TARGET STATUS ============================== //
#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentTargetStatus {
    #[default]
    Staged,
    Deployed,
    #[serde(other)]
    Archived,
}

impl<'de> Deserialize<'de> for DeploymentTargetStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let default = DeploymentTargetStatus::default();
        match s.as_str() {
            "staged" => Ok(DeploymentTargetStatus::Staged),
            "deployed" => Ok(DeploymentTargetStatus::Deployed),
            "archived" => Ok(DeploymentTargetStatus::Archived),
            status => {
                warn!(
                    "deployment target status '{}' is not valid, defaulting to {:?}",
                    status, default
                );
                Ok(default)
            }
        }
    }
}

impl DeploymentTargetStatus {
    pub fn variants() -> Vec<DeploymentTargetStatus> {
        vec![
            DeploymentTargetStatus::Staged,
            DeploymentTargetStatus::Deployed,
            DeploymentTargetStatus::Archived,
        ]
    }

    pub fn from_backend(
        target_status: &openapi_client::models::DeploymentTargetStatus,
    ) -> DeploymentTargetStatus {
        match target_status {
            openapi_client::models::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_STAGED => {
                DeploymentTargetStatus::Staged
            }
            openapi_client::models::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_DEPLOYED => {
                DeploymentTargetStatus::Deployed
            }
            openapi_client::models::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_ARCHIVED => {
                DeploymentTargetStatus::Archived
            }
        }
    }

    pub fn to_backend(
        target_status: &DeploymentTargetStatus,
    ) -> openapi_client::models::DeploymentTargetStatus {
        match target_status {
            DeploymentTargetStatus::Staged => {
                openapi_client::models::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_STAGED
            }
            DeploymentTargetStatus::Deployed => {
                openapi_client::models::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_DEPLOYED
            }
            DeploymentTargetStatus::Archived => {
                openapi_client::models::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_ARCHIVED
            }
        }
    }
}

// ========================= DEPLOYMENT ACTIVITY STATUS ============================= //
#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentActivityStatus {
    #[default]
    Drifted,
    Staged,
    Queued,
    Deployed,
    Archived,
}

impl<'de> Deserialize<'de> for DeploymentActivityStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let default = DeploymentActivityStatus::default();
        match s.as_str() {
            "drifted" => Ok(DeploymentActivityStatus::Drifted),
            "staged" => Ok(DeploymentActivityStatus::Staged),
            "queued" => Ok(DeploymentActivityStatus::Queued),
            "deployed" => Ok(DeploymentActivityStatus::Deployed),
            "archived" => Ok(DeploymentActivityStatus::Archived),
            status => {
                warn!(
                    "deployment activity status '{}' is not valid, defaulting to {:?}",
                    status, default
                );
                Ok(default)
            }
        }
    }
}

impl DeploymentActivityStatus {
    pub fn variants() -> Vec<DeploymentActivityStatus> {
        vec![
            DeploymentActivityStatus::Drifted,
            DeploymentActivityStatus::Staged,
            DeploymentActivityStatus::Queued,
            DeploymentActivityStatus::Deployed,
            DeploymentActivityStatus::Archived,
        ]
    }

    pub fn from_backend(
        activity_status: &openapi_client::models::DeploymentActivityStatus,
    ) -> DeploymentActivityStatus {
        match activity_status {
            openapi_client::models::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_DRIFTED => {
                DeploymentActivityStatus::Drifted
            }
            openapi_client::models::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_STAGED => {
                DeploymentActivityStatus::Staged
            }
            openapi_client::models::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_QUEUED => {
                DeploymentActivityStatus::Queued
            }
            openapi_client::models::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_DEPLOYED => {
                DeploymentActivityStatus::Deployed
            }
            openapi_client::models::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_ARCHIVED => {
                DeploymentActivityStatus::Archived
            }
        }
    }

    pub fn to_backend(
        activity_status: &DeploymentActivityStatus,
    ) -> openapi_client::models::DeploymentActivityStatus {
        match activity_status {
            DeploymentActivityStatus::Drifted => {
                openapi_client::models::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_DRIFTED
            }
            DeploymentActivityStatus::Staged => {
                openapi_client::models::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_STAGED
            }
            DeploymentActivityStatus::Queued => {
                openapi_client::models::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_QUEUED
            }
            DeploymentActivityStatus::Deployed => {
                openapi_client::models::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_DEPLOYED
            }
            DeploymentActivityStatus::Archived => {
                openapi_client::models::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_ARCHIVED
            }
        }
    }
}

// =========================== DEPLOYMENT ERROR STATUS =============================== //
#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentErrorStatus {
    #[default]
    None,
    Failed,
    Retrying,
}

impl<'de> Deserialize<'de> for DeploymentErrorStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let default = DeploymentErrorStatus::default();
        match s.as_str() {
            "none" => Ok(DeploymentErrorStatus::None),
            "failed" => Ok(DeploymentErrorStatus::Failed),
            "retrying" => Ok(DeploymentErrorStatus::Retrying),
            status => {
                warn!(
                    "deployment error status '{}' is not valid, defaulting to {:?}",
                    status, default
                );
                Ok(default)
            }
        }
    }
}

impl DeploymentErrorStatus {
    pub fn variants() -> Vec<DeploymentErrorStatus> {
        vec![
            DeploymentErrorStatus::None,
            DeploymentErrorStatus::Failed,
            DeploymentErrorStatus::Retrying,
        ]
    }

    pub fn from_backend(
        error_status: &openapi_client::models::DeploymentErrorStatus,
    ) -> DeploymentErrorStatus {
        match error_status {
            openapi_client::models::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_NONE => {
                DeploymentErrorStatus::None
            }
            openapi_client::models::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_FAILED => {
                DeploymentErrorStatus::Failed
            }
            openapi_client::models::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_RETRYING => {
                DeploymentErrorStatus::Retrying
            }
        }
    }

    pub fn to_backend(
        error_status: &DeploymentErrorStatus,
    ) -> openapi_client::models::DeploymentErrorStatus {
        match error_status {
            DeploymentErrorStatus::None => {
                openapi_client::models::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_NONE
            }
            DeploymentErrorStatus::Failed => {
                openapi_client::models::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_FAILED
            }
            DeploymentErrorStatus::Retrying => {
                openapi_client::models::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_RETRYING
            }
        }
    }
}

// =============================== DEPLOYMENT STATUS ================================ //
#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentStatus {
    #[default]
    Drifted,
    Staged,
    Queued,
    Deployed,
    Archived,
    Failed,
    Retrying,
}

impl<'de> Deserialize<'de> for DeploymentStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let default = DeploymentStatus::default();
        match s.as_str() {
            "drifted" => Ok(DeploymentStatus::Drifted),
            "staged" => Ok(DeploymentStatus::Staged),
            "queued" => Ok(DeploymentStatus::Queued),
            "deployed" => Ok(DeploymentStatus::Deployed),
            "archived" => Ok(DeploymentStatus::Archived),
            "failed" => Ok(DeploymentStatus::Failed),
            "retrying" => Ok(DeploymentStatus::Retrying),
            status => {
                warn!(
                    "deployment status '{}' is not valid, defaulting to {:?}",
                    status, default
                );
                Ok(default)
            }
        }
    }
}

impl DeploymentStatus {
    pub fn variants() -> Vec<DeploymentStatus> {
        vec![
            DeploymentStatus::Drifted,
            DeploymentStatus::Staged,
            DeploymentStatus::Queued,
            DeploymentStatus::Deployed,
            DeploymentStatus::Archived,
            DeploymentStatus::Failed,
            DeploymentStatus::Retrying,
        ]
    }

    pub fn from_backend(status: &openapi_client::models::DeploymentStatus) -> DeploymentStatus {
        match status {
            openapi_client::models::DeploymentStatus::DEPLOYMENT_STATUS_DRIFTED => {
                DeploymentStatus::Drifted
            }
            openapi_client::models::DeploymentStatus::DEPLOYMENT_STATUS_STAGED => {
                DeploymentStatus::Staged
            }
            openapi_client::models::DeploymentStatus::DEPLOYMENT_STATUS_QUEUED => {
                DeploymentStatus::Queued
            }
            openapi_client::models::DeploymentStatus::DEPLOYMENT_STATUS_DEPLOYED => {
                DeploymentStatus::Deployed
            }
            openapi_client::models::DeploymentStatus::DEPLOYMENT_STATUS_ARCHIVED => {
                DeploymentStatus::Archived
            }
            openapi_client::models::DeploymentStatus::DEPLOYMENT_STATUS_FAILED => {
                DeploymentStatus::Failed
            }
            openapi_client::models::DeploymentStatus::DEPLOYMENT_STATUS_RETRYING => {
                DeploymentStatus::Retrying
            }
        }
    }

    pub fn to_backend(status: &DeploymentStatus) -> openapi_client::models::DeploymentStatus {
        match status {
            DeploymentStatus::Drifted => {
                openapi_client::models::DeploymentStatus::DEPLOYMENT_STATUS_DRIFTED
            }
            DeploymentStatus::Staged => {
                openapi_client::models::DeploymentStatus::DEPLOYMENT_STATUS_STAGED
            }
            DeploymentStatus::Queued => {
                openapi_client::models::DeploymentStatus::DEPLOYMENT_STATUS_QUEUED
            }
            DeploymentStatus::Deployed => {
                openapi_client::models::DeploymentStatus::DEPLOYMENT_STATUS_DEPLOYED
            }
            DeploymentStatus::Archived => {
                openapi_client::models::DeploymentStatus::DEPLOYMENT_STATUS_ARCHIVED
            }
            DeploymentStatus::Failed => {
                openapi_client::models::DeploymentStatus::DEPLOYMENT_STATUS_FAILED
            }
            DeploymentStatus::Retrying => {
                openapi_client::models::DeploymentStatus::DEPLOYMENT_STATUS_RETRYING
            }
        }
    }

    pub fn from_activity_and_error(
        activity_status: &DeploymentActivityStatus,
        error_status: &DeploymentErrorStatus,
    ) -> DeploymentStatus {
        match error_status {
            DeploymentErrorStatus::None => match activity_status {
                DeploymentActivityStatus::Drifted => DeploymentStatus::Drifted,
                DeploymentActivityStatus::Staged => DeploymentStatus::Staged,
                DeploymentActivityStatus::Queued => DeploymentStatus::Queued,
                DeploymentActivityStatus::Deployed => DeploymentStatus::Deployed,
                DeploymentActivityStatus::Archived => DeploymentStatus::Archived,
            },
            DeploymentErrorStatus::Retrying => DeploymentStatus::Retrying,
            DeploymentErrorStatus::Failed => DeploymentStatus::Failed,
        }
    }
}

// ================================ DEPLOYMENT ====================================== //
pub type DeploymentID = String;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Deployment {
    pub id: String,
    pub description: String,
    pub status: DeploymentStatus,
    pub activity_status: DeploymentActivityStatus,
    pub error_status: DeploymentErrorStatus,
    pub target_status: DeploymentTargetStatus,
    pub device_id: String,
    pub release_id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    // Agent-side fields for retry logic (not from backend)
    pub attempts: u32,
    pub cooldown_ends_at: Option<DateTime<Utc>>,
    // Config instance IDs (not full objects)
    pub config_instance_ids: Vec<super::config_instance::ConfigInstanceID>,
}

impl Default for Deployment {
    fn default() -> Self {
        Self {
            id: format!("unknown-{}", Uuid::new_v4()),
            description: String::new(),
            status: DeploymentStatus::Staged,
            activity_status: DeploymentActivityStatus::Staged,
            error_status: DeploymentErrorStatus::None,
            target_status: DeploymentTargetStatus::Staged,
            device_id: format!("unknown-{}", Uuid::new_v4()),
            release_id: format!("unknown-{}", Uuid::new_v4()),
            created_at: DateTime::<Utc>::UNIX_EPOCH,
            updated_at: DateTime::<Utc>::UNIX_EPOCH,
            attempts: 0,
            cooldown_ends_at: None,
            config_instance_ids: Vec::new(),
        }
    }
}

impl Deployment {
    pub fn from_backend(backend_deployment: openapi_client::models::Deployment) -> Deployment {
        Deployment {
            id: backend_deployment.id,
            description: backend_deployment.description,
            status: DeploymentStatus::from_backend(&backend_deployment.status),
            activity_status: DeploymentActivityStatus::from_backend(&backend_deployment.activity_status),
            error_status: DeploymentErrorStatus::from_backend(&backend_deployment.error_status),
            target_status: DeploymentTargetStatus::from_backend(&backend_deployment.target_status),
            device_id: backend_deployment.device_id,
            release_id: backend_deployment.release_id,
            created_at: backend_deployment
                .created_at
                .parse::<DateTime<Utc>>()
                .unwrap_or_else(|_| DateTime::<Utc>::UNIX_EPOCH),
            updated_at: backend_deployment
                .updated_at
                .parse::<DateTime<Utc>>()
                .unwrap_or_else(|_| DateTime::<Utc>::UNIX_EPOCH),
            attempts: 0,
            cooldown_ends_at: None,
            config_instance_ids: backend_deployment
                .config_instances
                .map(|instances| instances.into_iter().map(|inst| inst.id).collect())
                .unwrap_or_default(),
        }
    }

    pub fn status(&self) -> DeploymentStatus {
        DeploymentStatus::from_activity_and_error(&self.activity_status, &self.error_status)
    }

    pub fn is_in_cooldown(&self) -> bool {
        match self.cooldown_ends_at {
            Some(cooldown_ends_at) => Utc::now() < cooldown_ends_at,
            None => false,
        }
    }

    pub fn set_cooldown(&mut self, cooldown: TimeDelta) {
        self.cooldown_ends_at = Some(Utc::now() + cooldown);
    }

    pub fn attempts(&self) -> u32 {
        self.attempts
    }
}

impl<'de> Deserialize<'de> for Deployment {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        pub struct DeserializeDeployment {
            id: String,
            description: String,
            status: DeploymentStatus,
            activity_status: DeploymentActivityStatus,
            error_status: DeploymentErrorStatus,
            target_status: DeploymentTargetStatus,
            device_id: String,
            release_id: String,
            created_at: Option<DateTime<Utc>>,
            updated_at: Option<DateTime<Utc>>,
            config_instance_ids: Vec<super::config_instance::ConfigInstanceID>,
        }

        let result = DeserializeDeployment::deserialize(deserializer)?;
        let default = Deployment::default();

        Ok(Deployment {
            id: result.id,
            description: result.description,
            status: result.status,
            activity_status: result.activity_status,
            error_status: result.error_status,
            target_status: result.target_status,
            device_id: result.device_id,
            release_id: result.release_id,
            created_at: result.created_at.unwrap_or_else(|| {
                deserialize_error!("deployment", "created_at", default.created_at)
            }),
            updated_at: result.updated_at.unwrap_or_else(|| {
                deserialize_error!("deployment", "updated_at", default.updated_at)
            }),
            attempts: 0,
            cooldown_ends_at: None,
            config_instance_ids: result.config_instance_ids,
        })
    }
}

