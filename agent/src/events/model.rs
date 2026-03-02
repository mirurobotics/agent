// internal crates
use crate::models;

// external crates
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ================================= ENVELOPE ====================================== //

/// A versioned event envelope. Every event emitted through the SSE stream uses
/// this shape. The `id` field is assigned by the EventHub and is monotonically
/// increasing across the lifetime of the event log.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Envelope {
    /// Monotonic event ID (assigned by the store).
    pub id: u64,
    /// Event type string, e.g. `deployment.deployed`.
    #[serde(rename = "type")]
    pub event_type: String,
    /// Schema version for payload migration.
    pub schema_version: u32,
    /// When the event occurred (UTC).
    pub occurred_at: DateTime<Utc>,
    /// Device that produced this event.
    pub device_id: String,
    /// The resource this event is about.
    pub subject: Subject,
    /// Event-specific payload.
    pub data: serde_json::Value,
}

/// Identifies the resource an event relates to.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Subject {
    /// Resource type, e.g. `"deployment"` or `"device"`.
    #[serde(rename = "type")]
    pub resource_type: String,
    /// Resource identifier.
    pub id: String,
}

// ================================ EVENT TYPES ==================================== //

pub const DEPLOYMENT_DEPLOYED: &str = "deployment.deployed";
pub const DEPLOYMENT_REMOVED: &str = "deployment.removed";
pub const SYNC_COMPLETED: &str = "sync.completed";
pub const SYNC_FAILED: &str = "sync.failed";

/// Current schema version for all event payloads.
pub const SCHEMA_VERSION: u32 = 1;

// =============================== PAYLOAD TYPES =================================== //

/// Payload for `deployment.deployed` and `deployment.removed` events.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeploymentEventData {
    pub target_status: models::DplTarget,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deployed_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<DateTime<Utc>>,
}

/// Payload for `sync.completed` events.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncCompletedData {
    pub last_synced_at: DateTime<Utc>,
}

/// Payload for `sync.failed` events.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncFailedData {
    pub is_network_conn_err: bool,
    pub err_streak: u32,
}

// =============================== CONSTRUCTORS ==================================== //

impl Envelope {
    fn new_with_data(
        event_type: &str,
        device_id: &str,
        subject: Subject,
        data: impl Serialize,
    ) -> Self {
        Self {
            id: 0,
            event_type: event_type.to_string(),
            schema_version: SCHEMA_VERSION,
            occurred_at: Utc::now(),
            device_id: device_id.to_string(),
            subject,
            data: serde_json::to_value(data).unwrap_or_default(),
        }
    }

    fn deployment_event(
        event_type: &str,
        device_id: &str,
        deployment: &models::Deployment,
    ) -> Self {
        let data = DeploymentEventData {
            target_status: deployment.target_status,
            deployed_at: deployment.deployed_at,
            archived_at: deployment.archived_at,
        };
        Self::new_with_data(
            event_type,
            device_id,
            Subject {
                resource_type: "deployment".to_string(),
                id: deployment.id.clone(),
            },
            data,
        )
    }

    /// Build a `deployment.deployed` envelope.
    pub fn deployment_deployed(device_id: &str, deployment: &models::Deployment) -> Self {
        Self::deployment_event(DEPLOYMENT_DEPLOYED, device_id, deployment)
    }

    /// Build a `deployment.removed` envelope.
    pub fn deployment_removed(device_id: &str, deployment: &models::Deployment) -> Self {
        Self::deployment_event(DEPLOYMENT_REMOVED, device_id, deployment)
    }

    /// Build a sync.completed envelope.
    pub fn sync_completed(device_id: &str, last_synced_at: DateTime<Utc>) -> Self {
        let data = SyncCompletedData { last_synced_at };
        Self::new_with_data(
            SYNC_COMPLETED,
            device_id,
            Subject {
                resource_type: "device".to_string(),
                id: device_id.to_string(),
            },
            data,
        )
    }

    /// Build a sync.failed envelope.
    pub fn sync_failed(device_id: &str, is_network_conn_err: bool, err_streak: u32) -> Self {
        let data = SyncFailedData {
            is_network_conn_err,
            err_streak,
        };
        Self::new_with_data(
            SYNC_FAILED,
            device_id,
            Subject {
                resource_type: "device".to_string(),
                id: device_id.to_string(),
            },
            data,
        )
    }
}
