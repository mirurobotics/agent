// internal crates
use crate::deserialize_error;
use crate::models::config_instance::CfgInstID;
use crate::models::Patch;
use backend_api::models as backend_client;
use device_api::models as agent_server;

// external crates
use chrono::{DateTime, TimeDelta, Utc};
use serde::Deserialize;
use serde::Serialize;
use tracing::warn;
use uuid::Uuid;

macro_rules! impl_status_enum_with_backend {
    (
        enum $name:ident,
        default: $default:ident,
        label: $label:expr,
        log: $log_macro:ident,
        agent_type: $agent_type:ty,
        backend_type: $backend_type:ty,
        mappings: [
            $(
                $variant:ident => $wire:literal =>
                    $agent_value:expr =>
                    $backend_value:path
            ),+ $(,)?
        ]
    ) => {
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let s = String::deserialize(deserializer)?;
                let default = $name::$default;
                match s.as_str() {
                    $(
                        $wire => Ok($name::$variant),
                    )+
                    status => {
                        $log_macro!(
                            "{} '{}' is not valid, defaulting to {:?}",
                            $label, status, default
                        );
                        Ok(default)
                    }
                }
            }
        }

        impl $name {
            pub fn variants() -> Vec<$name> {
                vec![$($name::$variant),+]
            }
        }

        impl From<&$name> for $agent_type {
            fn from(status: &$name) -> Self {
                match status {
                    $(
                        $name::$variant => $agent_value,
                    )+
                }
            }
        }

        impl From<&$name> for $backend_type {
            fn from(status: &$name) -> Self {
                match status {
                    $(
                        $name::$variant => $backend_value,
                    )+
                }
            }
        }

        impl From<&$backend_type> for $name {
            fn from(status: &$backend_type) -> $name {
                match status {
                    $(
                        $backend_value => $name::$variant,
                    )+
                }
            }
        }
    };
}

// =========================== DEPLOYMENT TARGET STATUS ============================== //
#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum DplTarget {
    #[default]
    Staged,
    Deployed,
    Archived,
}

impl_status_enum_with_backend!(
    enum DplTarget,
    default: Staged,
    label: "deployment target status",
    log: warn,
    agent_type: agent_server::DeploymentTargetStatus,
    backend_type: backend_client::DeploymentTargetStatus,
    mappings: [
        Staged => "staged" =>
            agent_server::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_STAGED =>
            backend_client::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_STAGED,
        Deployed => "deployed" =>
            agent_server::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_DEPLOYED =>
            backend_client::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_DEPLOYED,
        Archived => "archived" =>
            agent_server::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_ARCHIVED =>
            backend_client::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_ARCHIVED,
    ]
);

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

impl_status_enum_with_backend!(
    enum DplActivity,
    default: Drifted,
    label: "deployment activity status",
    log: warn,
    agent_type: agent_server::DeploymentActivityStatus,
    backend_type: backend_client::DeploymentActivityStatus,
    mappings: [
        Drifted => "drifted" =>
            agent_server::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_DRIFTED =>
            backend_client::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_DRIFTED,
        Staged => "staged" =>
            agent_server::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_STAGED =>
            backend_client::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_STAGED,
        Queued => "queued" =>
            agent_server::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_QUEUED =>
            backend_client::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_QUEUED,
        Deployed => "deployed" =>
            agent_server::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_DEPLOYED =>
            backend_client::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_DEPLOYED,
        Archived => "archived" =>
            agent_server::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_ARCHIVED =>
            backend_client::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_ARCHIVED,
    ]
);

// =========================== DEPLOYMENT ERROR STATUS =============================== //
#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum DplErrStatus {
    #[default]
    None,
    Failed,
    Retrying,
}

impl_status_enum_with_backend!(
    enum DplErrStatus,
    default: None,
    label: "deployment error status",
    log: warn,
    agent_type: agent_server::DeploymentErrorStatus,
    backend_type: backend_client::DeploymentErrorStatus,
    mappings: [
        None => "none" =>
            agent_server::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_NONE =>
            backend_client::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_NONE,
        Failed => "failed" =>
            agent_server::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_FAILED =>
            backend_client::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_FAILED,
        Retrying => "retrying" =>
            agent_server::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_RETRYING =>
            backend_client::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_RETRYING,
    ]
);

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

impl DplStatus {
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

impl_status_enum_with_backend!(
    enum DplStatus,
    default: Drifted,
    label: "deployment status",
    log: warn,
    agent_type: agent_server::DeploymentStatus,
    backend_type: backend_client::DeploymentStatus,
    mappings: [
        Drifted => "drifted" =>
            agent_server::DeploymentStatus::DEPLOYMENT_STATUS_DRIFTED =>
            backend_client::DeploymentStatus::DEPLOYMENT_STATUS_DRIFTED,
        Staged => "staged" =>
            agent_server::DeploymentStatus::DEPLOYMENT_STATUS_STAGED =>
            backend_client::DeploymentStatus::DEPLOYMENT_STATUS_STAGED,
        Queued => "queued" =>
            agent_server::DeploymentStatus::DEPLOYMENT_STATUS_QUEUED =>
            backend_client::DeploymentStatus::DEPLOYMENT_STATUS_QUEUED,
        Deployed => "deployed" =>
            agent_server::DeploymentStatus::DEPLOYMENT_STATUS_DEPLOYED =>
            backend_client::DeploymentStatus::DEPLOYMENT_STATUS_DEPLOYED,
        Archived => "archived" =>
            agent_server::DeploymentStatus::DEPLOYMENT_STATUS_ARCHIVED =>
            backend_client::DeploymentStatus::DEPLOYMENT_STATUS_ARCHIVED,
        Failed => "failed" =>
            agent_server::DeploymentStatus::DEPLOYMENT_STATUS_FAILED =>
            backend_client::DeploymentStatus::DEPLOYMENT_STATUS_FAILED,
        Retrying => "retrying" =>
            agent_server::DeploymentStatus::DEPLOYMENT_STATUS_RETRYING =>
            backend_client::DeploymentStatus::DEPLOYMENT_STATUS_RETRYING,
    ]
);

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
    // Agent-side timestamps pushed to backend.
    // These are independent historical watermarks: each records the last time the
    // deployment entered that state. Both may be set simultaneously after a
    // deploy→archive→redeploy cycle; the current state is always determined by
    // `activity_status`, not by which timestamp is present.
    pub deployed_at: Option<DateTime<Utc>>,
    pub archived_at: Option<DateTime<Utc>>,
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
            deployed_at: None,
            archived_at: None,
            config_instance_ids: Vec::new(),
        }
    }
}

impl Deployment {
    pub fn from_backend(
        deployment: backend_client::Deployment,
        config_instance_ids: Vec<String>,
    ) -> Deployment {
        Deployment {
            id: deployment.id,
            description: deployment.description,
            activity_status: (&deployment.activity_status).into(),
            error_status: (&deployment.error_status).into(),
            target_status: (&deployment.target_status).into(),
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
            deployed_at: None,
            archived_at: None,
            config_instance_ids,
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
            deployed_at: Option<DateTime<Utc>>,
            archived_at: Option<DateTime<Utc>>,
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
            deployed_at: result.deployed_at,
            archived_at: result.archived_at,
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
    pub deployed_at: Option<DateTime<Utc>>,
    pub archived_at: Option<DateTime<Utc>>,
}

impl Updates {
    pub fn empty() -> Self {
        Self {
            activity_status: None,
            error_status: None,
            attempts: None,
            cooldown: None,
            deployed_at: None,
            archived_at: None,
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
        if let Some(deployed_at) = patch.deployed_at {
            self.deployed_at = Some(deployed_at);
        }
        if let Some(archived_at) = patch.archived_at {
            self.archived_at = Some(archived_at);
        }
    }
}
