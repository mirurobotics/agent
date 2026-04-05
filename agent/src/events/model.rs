// standard crates
use std::collections::HashSet;

// internal crates
use crate::events::errors::EventsErr;
use crate::models;
use device_api::models as device_server;

// external crates
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub type EventTypeFilter = HashSet<String>;

pub const DEPLOYMENT_DEPLOYED: &str = "deployment.deployed";
pub const DEPLOYMENT_REMOVED: &str = "deployment.removed";

pub type DeploymentDeployedEvent = device_server::DeploymentDeployedEvent;
pub type DeploymentRemovedEvent = device_server::DeploymentRemovedEvent;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: i64,
    #[serde(rename = "type")]
    pub event_type: String,
    pub occurred_at: DateTime<Utc>,
    pub data: serde_json::Value,
}

impl Event {
    pub(crate) fn new(id: i64, args: EventArgs) -> Self {
        Self {
            id,
            event_type: args.event_type,
            occurred_at: args.occurred_at,
            data: args.data,
        }
    }
}

/// Input type for creating a new event. The store assigns `id`.
#[derive(Debug, PartialEq)]
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
            DEPLOYMENT_DEPLOYED,
            DeploymentDeployedEvent {
                deployment_id: deployment.id.clone(),
                release_id: deployment.release_id.clone(),
                status: (&deployment.status()).into(),
                activity_status: (&deployment.activity_status).into(),
                error_status: (&deployment.error_status).into(),
                target_status: (&deployment.target_status).into(),
                deployed_at: deployment.deployed_at.map(|dt| dt.to_rfc3339()),
            },
        )
    }

    pub fn removed(deployment: &models::Deployment) -> Result<Self, EventsErr> {
        Self::new(
            DEPLOYMENT_REMOVED,
            DeploymentRemovedEvent {
                deployment_id: deployment.id.clone(),
                release_id: deployment.release_id.clone(),
                status: (&deployment.status()).into(),
                activity_status: (&deployment.activity_status).into(),
                error_status: (&deployment.error_status).into(),
                target_status: (&deployment.target_status).into(),
                archived_at: deployment.archived_at.map(|dt| dt.to_rfc3339()),
            },
        )
    }
}
