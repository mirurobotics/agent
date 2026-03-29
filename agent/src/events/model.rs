// internal crates
use crate::events::errors::EventsErr;
use crate::models;
use device_api::models as device_server;

// external crates
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const DEPLOYMENT_DEPLOYED_BETA1: &str = "deployment.deployed.beta1";
pub const DEPLOYMENT_REMOVED_BETA1: &str = "deployment.removed.beta1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: u64,
    #[serde(rename = "type")]
    pub event_type: String,
    pub occurred_at: DateTime<Utc>,
    pub data: serde_json::Value,
}

impl Event {
    pub(crate) fn from_new_event(id: u64, event: EventArgs) -> Self {
        Self {
            id,
            event_type: event.event_type,
            occurred_at: event.occurred_at,
            data: event.data,
        }
    }
}

/// Input type for creating a new event. The store assigns `id`.
pub struct EventArgs {
    pub event_type: String,
    pub occurred_at: DateTime<Utc>,
    pub data: serde_json::Value,
}

impl EventArgs {
    fn new(event_type: &str, data: impl Serialize) -> Result<Self, EventsErr> {
        Ok(Self {
            event_type: event_type.to_string(),
            occurred_at: Utc::now(),
            data: serde_json::to_value(data)?,
        })
    }

    pub fn deployment_deployed(deployment: &models::Deployment) -> Result<Self, EventsErr> {
        Self::new(
            DEPLOYMENT_DEPLOYED_BETA1,
            device_server::DeploymentDeployedBeta1Event {
                deployment_id: deployment.id.clone(),
                activity_status: status_str(&deployment.activity_status),
                target_status: status_str(&deployment.target_status),
                deployed_at: deployment.deployed_at.map(|dt| dt.to_rfc3339()),
            },
        )
    }

    pub fn deployment_removed(deployment: &models::Deployment) -> Result<Self, EventsErr> {
        Self::new(
            DEPLOYMENT_REMOVED_BETA1,
            device_server::DeploymentRemovedBeta1Event {
                deployment_id: deployment.id.clone(),
                activity_status: status_str(&deployment.activity_status),
                target_status: status_str(&deployment.target_status),
                archived_at: deployment.archived_at.map(|dt| dt.to_rfc3339()),
            },
        )
    }
}

/// Serialize a serde-compatible enum variant to its wire-format string.
fn status_str(status: &impl Serialize) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}
