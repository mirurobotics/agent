// standard crates
use std::collections::HashSet;

// internal crates
use crate::events::errors::{EventsErr, UnknownEventTypeErr};
use crate::models;

// external crates
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub type EventTypeFilter = HashSet<String>;

pub const DEPLOYMENT_DEPLOYED: &str = "deployment.deployed";
pub const DEPLOYMENT_REMOVED: &str = "deployment.removed";

// The persisted event-log format. Each Device API version converts it to its
// own wire type, so a change here must keep old stored events readable.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct DeploymentDeployedEvent {
    pub deployment_id: String,
    pub release_id: String,
    pub status: models::DplStatus,
    pub activity_status: models::DplActivity,
    pub error_status: models::DplErrStatus,
    pub target_status: models::DplTarget,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deployed_at: Option<String>,
}

// The persisted event-log format. Each Device API version converts it to its
// own wire type, so a change here must keep old stored events readable.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct DeploymentRemovedEvent {
    pub deployment_id: String,
    pub release_id: String,
    pub status: models::DplStatus,
    pub activity_status: models::DplActivity,
    pub error_status: models::DplErrStatus,
    pub target_status: models::DplTarget,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<String>,
}

/// A stored event's data, parsed by its type.
#[derive(Debug, PartialEq)]
pub enum EventPayload {
    Deployed(DeploymentDeployedEvent),
    Removed(DeploymentRemovedEvent),
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

    pub fn payload(&self) -> Result<EventPayload, EventsErr> {
        let data = self.data.clone();
        match self.event_type.as_str() {
            DEPLOYMENT_DEPLOYED => Ok(EventPayload::Deployed(serde_json::from_value(data)?)),
            DEPLOYMENT_REMOVED => Ok(EventPayload::Removed(serde_json::from_value(data)?)),
            other => {
                let err = UnknownEventTypeErr(other.to_string());
                Err(EventsErr::UnknownEventTypeErr(err))
            }
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
