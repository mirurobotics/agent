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
    pub(crate) fn new(id: u64, args: EventArgs) -> Self {
        Self {
            id,
            event_type: args.event_type,
            occurred_at: args.occurred_at,
            data: args.data,
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

    pub fn deployed(deployment: &models::Deployment) -> Result<Self, EventsErr> {
        Self::new(
            DEPLOYMENT_DEPLOYED_BETA1,
            device_server::DeploymentDeployedBeta1Event {
                deployment_id: deployment.id.clone(),
                activity_status: deployment.activity_status.as_str().to_owned(),
                target_status: deployment.target_status.as_str().to_owned(),
                deployed_at: deployment.deployed_at.map(|dt| dt.to_rfc3339()),
            },
        )
    }

    pub fn removed(deployment: &models::Deployment) -> Result<Self, EventsErr> {
        Self::new(
            DEPLOYMENT_REMOVED_BETA1,
            device_server::DeploymentRemovedBeta1Event {
                deployment_id: deployment.id.clone(),
                activity_status: deployment.activity_status.as_str().to_owned(),
                target_status: deployment.target_status.as_str().to_owned(),
                archived_at: deployment.archived_at.map(|dt| dt.to_rfc3339()),
            },
        )
    }
}
