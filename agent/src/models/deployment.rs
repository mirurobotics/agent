// internal crates
use crate::deserialize_error;
use crate::models::config_instance::CfgInstID;
use crate::models::Patch;
use openapi_client::models as backend_client;

// external crates
use chrono::{DateTime, TimeDelta, Utc};
use serde::Deserialize;
use serde::Serialize;
use tracing::warn;
use uuid::Uuid;

// =========================== DEPLOYMENT TARGET STATUS ============================== //
#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum DplTarget {
    #[default]
    Staged,
    Deployed,
    #[serde(other)]
    Archived,
}

impl<'de> Deserialize<'de> for DplTarget {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let default = DplTarget::default();
        match s.as_str() {
            "staged" => Ok(DplTarget::Staged),
            "deployed" => Ok(DplTarget::Deployed),
            "archived" => Ok(DplTarget::Archived),
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

impl DplTarget {
    pub fn variants() -> Vec<DplTarget> {
        vec![DplTarget::Staged, DplTarget::Deployed, DplTarget::Archived]
    }

    pub fn from_backend(target_status: &backend_client::DeploymentTargetStatus) -> DplTarget {
        match target_status {
            backend_client::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_STAGED => {
                DplTarget::Staged
            }
            backend_client::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_DEPLOYED => {
                DplTarget::Deployed
            }
            backend_client::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_ARCHIVED => {
                DplTarget::Archived
            }
        }
    }

    pub fn to_backend(target_status: &DplTarget) -> backend_client::DeploymentTargetStatus {
        match target_status {
            DplTarget::Staged => {
                backend_client::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_STAGED
            }
            DplTarget::Deployed => {
                backend_client::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_DEPLOYED
            }
            DplTarget::Archived => {
                backend_client::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_ARCHIVED
            }
        }
    }
}

// ========================= DEPLOYMENT ACTIVITY STATUS ============================= //
#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum DplActivity {
    #[default]
    Drifted,
    Staged,
    Queued,
    Deployed,
    Archived,
}

impl<'de> Deserialize<'de> for DplActivity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let default = DplActivity::default();
        match s.as_str() {
            "drifted" => Ok(DplActivity::Drifted),
            "staged" => Ok(DplActivity::Staged),
            "queued" => Ok(DplActivity::Queued),
            "deployed" => Ok(DplActivity::Deployed),
            "archived" => Ok(DplActivity::Archived),
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

impl DplActivity {
    pub fn variants() -> Vec<DplActivity> {
        vec![
            DplActivity::Drifted,
            DplActivity::Staged,
            DplActivity::Queued,
            DplActivity::Deployed,
            DplActivity::Archived,
        ]
    }

    pub fn from_backend(activity_status: &backend_client::DeploymentActivityStatus) -> DplActivity {
        match activity_status {
            backend_client::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_DRIFTED => {
                DplActivity::Drifted
            }
            backend_client::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_STAGED => {
                DplActivity::Staged
            }
            backend_client::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_QUEUED => {
                DplActivity::Queued
            }
            backend_client::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_DEPLOYED => {
                DplActivity::Deployed
            }
            backend_client::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_ARCHIVED => {
                DplActivity::Archived
            }
        }
    }

    pub fn to_backend(activity_status: &DplActivity) -> backend_client::DeploymentActivityStatus {
        match activity_status {
            DplActivity::Drifted => {
                backend_client::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_DRIFTED
            }
            DplActivity::Staged => {
                backend_client::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_STAGED
            }
            DplActivity::Queued => {
                backend_client::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_QUEUED
            }
            DplActivity::Deployed => {
                backend_client::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_DEPLOYED
            }
            DplActivity::Archived => {
                backend_client::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_ARCHIVED
            }
        }
    }
}

// =========================== DEPLOYMENT ERROR STATUS =============================== //
#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum DplErrStatus {
    #[default]
    None,
    Failed,
    Retrying,
}

impl<'de> Deserialize<'de> for DplErrStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let default = DplErrStatus::default();
        match s.as_str() {
            "none" => Ok(DplErrStatus::None),
            "failed" => Ok(DplErrStatus::Failed),
            "retrying" => Ok(DplErrStatus::Retrying),
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

impl DplErrStatus {
    pub fn variants() -> Vec<DplErrStatus> {
        vec![
            DplErrStatus::None,
            DplErrStatus::Failed,
            DplErrStatus::Retrying,
        ]
    }

    pub fn from_backend(error_status: &backend_client::DeploymentErrorStatus) -> DplErrStatus {
        match error_status {
            backend_client::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_NONE => {
                DplErrStatus::None
            }
            backend_client::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_FAILED => {
                DplErrStatus::Failed
            }
            backend_client::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_RETRYING => {
                DplErrStatus::Retrying
            }
        }
    }

    pub fn to_backend(error_status: &DplErrStatus) -> backend_client::DeploymentErrorStatus {
        match error_status {
            DplErrStatus::None => {
                backend_client::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_NONE
            }
            DplErrStatus::Failed => {
                backend_client::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_FAILED
            }
            DplErrStatus::Retrying => {
                backend_client::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_RETRYING
            }
        }
    }
}

// =============================== DEPLOYMENT STATUS ================================ //
#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum DplStatus {
    #[default]
    Drifted,
    Staged,
    Queued,
    Deployed,
    Archived,
    Failed,
    Retrying,
}

impl<'de> Deserialize<'de> for DplStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let default = DplStatus::default();
        match s.as_str() {
            "drifted" => Ok(DplStatus::Drifted),
            "staged" => Ok(DplStatus::Staged),
            "queued" => Ok(DplStatus::Queued),
            "deployed" => Ok(DplStatus::Deployed),
            "archived" => Ok(DplStatus::Archived),
            "failed" => Ok(DplStatus::Failed),
            "retrying" => Ok(DplStatus::Retrying),
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

impl DplStatus {
    pub fn variants() -> Vec<DplStatus> {
        vec![
            DplStatus::Drifted,
            DplStatus::Staged,
            DplStatus::Queued,
            DplStatus::Deployed,
            DplStatus::Archived,
            DplStatus::Failed,
            DplStatus::Retrying,
        ]
    }

    pub fn from_backend(status: &backend_client::DeploymentStatus) -> DplStatus {
        match status {
            backend_client::DeploymentStatus::DEPLOYMENT_STATUS_DRIFTED => DplStatus::Drifted,
            backend_client::DeploymentStatus::DEPLOYMENT_STATUS_STAGED => DplStatus::Staged,
            backend_client::DeploymentStatus::DEPLOYMENT_STATUS_QUEUED => DplStatus::Queued,
            backend_client::DeploymentStatus::DEPLOYMENT_STATUS_DEPLOYED => DplStatus::Deployed,
            backend_client::DeploymentStatus::DEPLOYMENT_STATUS_ARCHIVED => DplStatus::Archived,
            backend_client::DeploymentStatus::DEPLOYMENT_STATUS_FAILED => DplStatus::Failed,
            backend_client::DeploymentStatus::DEPLOYMENT_STATUS_RETRYING => DplStatus::Retrying,
        }
    }

    pub fn to_backend(status: &DplStatus) -> backend_client::DeploymentStatus {
        match status {
            DplStatus::Drifted => backend_client::DeploymentStatus::DEPLOYMENT_STATUS_DRIFTED,
            DplStatus::Staged => backend_client::DeploymentStatus::DEPLOYMENT_STATUS_STAGED,
            DplStatus::Queued => backend_client::DeploymentStatus::DEPLOYMENT_STATUS_QUEUED,
            DplStatus::Deployed => backend_client::DeploymentStatus::DEPLOYMENT_STATUS_DEPLOYED,
            DplStatus::Archived => backend_client::DeploymentStatus::DEPLOYMENT_STATUS_ARCHIVED,
            DplStatus::Failed => backend_client::DeploymentStatus::DEPLOYMENT_STATUS_FAILED,
            DplStatus::Retrying => backend_client::DeploymentStatus::DEPLOYMENT_STATUS_RETRYING,
        }
    }

    pub fn from_activity_and_error(
        activity_status: &DplActivity,
        error_status: &DplErrStatus,
    ) -> DplStatus {
        match error_status {
            DplErrStatus::None => match activity_status {
                DplActivity::Drifted => DplStatus::Drifted,
                DplActivity::Staged => DplStatus::Staged,
                DplActivity::Queued => DplStatus::Queued,
                DplActivity::Deployed => DplStatus::Deployed,
                DplActivity::Archived => DplStatus::Archived,
            },
            DplErrStatus::Retrying => DplStatus::Retrying,
            DplErrStatus::Failed => DplStatus::Failed,
        }
    }
}

// ================================ DEPLOYMENT ====================================== //
pub type DeploymentID = String;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Deployment {
    pub id: String,
    pub description: String,
    pub activity_status: DplActivity,
    pub error_status: DplErrStatus,
    pub target_status: DplTarget,
    pub device_id: String,
    pub release_id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub config_instance_ids: Vec<CfgInstID>,
    // Agent-side fields for retry logic (not from backend)
    pub attempts: u32,
    pub cooldown_ends_at: DateTime<Utc>,
}

impl Default for Deployment {
    fn default() -> Self {
        Self {
            id: format!("unknown-{}", Uuid::new_v4()),
            description: String::new(),
            activity_status: DplActivity::Staged,
            error_status: DplErrStatus::None,
            target_status: DplTarget::Staged,
            device_id: format!("unknown-{}", Uuid::new_v4()),
            release_id: format!("unknown-{}", Uuid::new_v4()),
            created_at: DateTime::<Utc>::UNIX_EPOCH,
            updated_at: DateTime::<Utc>::UNIX_EPOCH,
            attempts: 0,
            cooldown_ends_at: DateTime::<Utc>::UNIX_EPOCH,
            config_instance_ids: Vec::new(),
        }
    }
}

impl Deployment {
    pub fn from_backend(deployment: backend_client::Deployment) -> Deployment {
        Deployment {
            id: deployment.id,
            description: deployment.description,
            activity_status: DplActivity::from_backend(&deployment.activity_status),
            error_status: DplErrStatus::from_backend(&deployment.error_status),
            target_status: DplTarget::from_backend(&deployment.target_status),
            device_id: deployment.device_id,
            release_id: deployment.release_id,
            created_at: deployment
                .created_at
                .parse::<DateTime<Utc>>()
                .unwrap_or(DateTime::<Utc>::UNIX_EPOCH),
            updated_at: deployment
                .updated_at
                .parse::<DateTime<Utc>>()
                .unwrap_or(DateTime::<Utc>::UNIX_EPOCH),
            attempts: 0,
            cooldown_ends_at: DateTime::<Utc>::UNIX_EPOCH,
            config_instance_ids: deployment
                .config_instances
                .map(|instances| instances.into_iter().map(|inst| inst.id).collect())
                .unwrap_or_default(),
        }
    }

    pub fn status(&self) -> DplStatus {
        DplStatus::from_activity_and_error(&self.activity_status, &self.error_status)
    }

    pub fn is_in_cooldown(&self) -> bool {
        Utc::now() < self.cooldown_ends_at
    }

    pub fn set_cooldown(&mut self, cooldown: TimeDelta) {
        self.cooldown_ends_at = Utc::now() + cooldown;
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
            activity_status: DplActivity,
            error_status: DplErrStatus,
            target_status: DplTarget,
            device_id: String,
            release_id: String,
            created_at: Option<DateTime<Utc>>,
            updated_at: Option<DateTime<Utc>>,
            attempts: Option<u32>,
            cooldown_ends_at: Option<DateTime<Utc>>,
            config_instance_ids: Vec<CfgInstID>,
        }

        let result = DeserializeDeployment::deserialize(deserializer)?;
        let default = Deployment::default();

        Ok(Deployment {
            id: result.id,
            description: result.description,
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
            attempts: result
                .attempts
                .unwrap_or_else(|| deserialize_error!("deployment", "attempts", default.attempts)),
            cooldown_ends_at: result
                .cooldown_ends_at
                .unwrap_or(DateTime::<Utc>::UNIX_EPOCH),
            config_instance_ids: result.config_instance_ids,
        })
    }
}

#[derive(Debug, PartialEq)]
pub struct Updates {
    pub activity_status: Option<DplActivity>,
    pub error_status: Option<DplErrStatus>,
    pub attempts: Option<u32>,
    pub cooldown: Option<TimeDelta>,
}

impl Updates {
    pub fn empty() -> Self {
        Self {
            activity_status: None,
            error_status: None,
            attempts: None,
            cooldown: None,
        }
    }
}

impl Patch<Updates> for Deployment {
    fn patch(&mut self, patch: Updates) {
        if let Some(activity_status) = patch.activity_status {
            self.activity_status = activity_status;
        }
        if let Some(error_status) = patch.error_status {
            self.error_status = error_status;
        }
        if let Some(attempts) = patch.attempts {
            self.attempts = attempts;
        }
        if let Some(cooldown) = patch.cooldown {
            self.set_cooldown(cooldown);
        }
    }
}
