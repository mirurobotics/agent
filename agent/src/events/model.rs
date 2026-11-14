// standard crates
use std::collections::HashSet;

// internal crates
use crate::events::errors::EventsErr;
use crate::models;

// external crates
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub type EventTypeFilter = HashSet<String>;

pub const DEPLOYMENT_DEPLOYED: &str = "deployment.deployed";
pub const DEPLOYMENT_REMOVED: &str = "deployment.removed";

// The persisted event-log format. Clients receive it unchanged, so it must not change.
#[derive(Debug, Serialize)]
struct DeploymentDeployedEvent {
    deployment_id: String,
    release_id: String,
    status: models::DplStatus,
    activity_status: models::DplActivity,
    error_status: models::DplErrStatus,
    target_status: models::DplTarget,
    #[serde(skip_serializing_if = "Option::is_none")]
    deployed_at: Option<String>,
}

// The persisted event-log format. Clients receive it unchanged, so it must not change.
#[derive(Debug, Serialize)]
struct DeploymentRemovedEvent {
    deployment_id: String,
    release_id: String,
    status: models::DplStatus,
    activity_status: models::DplActivity,
    error_status: models::DplErrStatus,
    target_status: models::DplTarget,
    #[serde(skip_serializing_if = "Option::is_none")]
    archived_at: Option<String>,
}

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
                status: deployment.status(),
                activity_status: deployment.activity_status,
                error_status: deployment.error_status,
                target_status: deployment.target_status,
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
                status: deployment.status(),
                activity_status: deployment.activity_status,
                error_status: deployment.error_status,
                target_status: deployment.target_status,
                archived_at: deployment.archived_at.map(|dt| dt.to_rfc3339()),
            },
        )
    }
}
