// internal crates
use crate::deserialize_error;
use crate::utils::Mergeable;

// external crates
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::error;

#[derive(Debug, Serialize, PartialEq, Eq, Clone, Hash)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum DeviceStatus {
    Online,
    #[default]
    Offline,
}

impl DeviceStatus {
    pub fn variants() -> Vec<DeviceStatus> {
        vec![DeviceStatus::Online, DeviceStatus::Offline]
    }

    pub fn to_sdk(device_status: &DeviceStatus) -> openapi_server::models::DeviceStatus {
        match device_status {
            DeviceStatus::Online => openapi_server::models::DeviceStatus::DEVICE_STATUS_ONLINE,
            DeviceStatus::Offline => openapi_server::models::DeviceStatus::DEVICE_STATUS_OFFLINE,
        }
    }
}

impl<'de> Deserialize<'de> for DeviceStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let default = DeviceStatus::default();
        match s.as_str() {
            "online" => Ok(DeviceStatus::Online),
            "offline" => Ok(DeviceStatus::Offline),
            status => {
                error!(
                    "device status '{}' is not valid, defaulting to {:?}",
                    status, default
                );
                Ok(default)
            }
        }
    }
}

#[derive(Debug, Serialize, PartialEq, Eq, Clone)]
pub struct Device {
    #[serde(rename = "device_id")]
    pub id: String,
    pub session_id: String,
    pub name: String,
    pub agent_version: String,
    pub activated: bool,
    pub status: DeviceStatus,
    pub last_synced_at: DateTime<Utc>,
    pub last_connected_at: DateTime<Utc>,
    pub last_disconnected_at: DateTime<Utc>,
}

impl Default for Device {
    fn default() -> Self {
        Self {
            id: "placeholder".to_string(),
            session_id: "placeholder".to_string(),
            name: "placeholder".to_string(),
            agent_version: "placeholder".to_string(),
            activated: false,
            status: DeviceStatus::Offline,
            last_synced_at: DateTime::<Utc>::UNIX_EPOCH,
            last_connected_at: DateTime::<Utc>::UNIX_EPOCH,
            last_disconnected_at: DateTime::<Utc>::UNIX_EPOCH,
        }
    }
}

impl<'de> Deserialize<'de> for Device {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct DeserializeAgent {
            // the old field name was device_id so we'll keep it for backwards
            // compatibility
            device_id: String,
            session_id: String,
            name: Option<String>,
            agent_version: Option<String>,
            activated: Option<bool>,
            status: Option<DeviceStatus>,
            last_synced_at: Option<DateTime<Utc>>,
            last_connected_at: Option<DateTime<Utc>>,
            last_disconnected_at: Option<DateTime<Utc>>,
        }

        let result = match DeserializeAgent::deserialize(deserializer) {
            Ok(agent) => agent,
            Err(e) => {
                error!("Error deserializing agent: {}", e);
                return Err(e);
            }
        };

        let default = Device::default();

        Ok(Device {
            id: result.device_id,
            session_id: result.session_id,
            name: result
                .name
                .unwrap_or_else(|| deserialize_error!("device", "name", default.name)),
            activated: result
                .activated
                .unwrap_or_else(|| deserialize_error!("device", "activated", default.activated)),
            agent_version: result.agent_version.unwrap_or_else(|| {
                deserialize_error!("device", "agent_version", default.agent_version)
            }),
            status: result
                .status
                .unwrap_or_else(|| deserialize_error!("device", "status", default.status)),
            last_synced_at: result.last_synced_at.unwrap_or_else(|| {
                deserialize_error!("device", "last_synced_at", default.last_synced_at)
            }),
            last_connected_at: result.last_connected_at.unwrap_or_else(|| {
                deserialize_error!("device", "last_connected_at", default.last_connected_at)
            }),
            last_disconnected_at: result.last_disconnected_at.unwrap_or_else(|| {
                deserialize_error!(
                    "device",
                    "last_disconnected_at",
                    default.last_disconnected_at
                )
            }),
        })
    }
}

impl Mergeable<Updates> for Device {
    fn merge(&mut self, updates: Updates) {
        if let Some(id) = updates.id {
            self.id = id;
        }
        if let Some(name) = updates.name {
            self.name = name;
        }
        if let Some(agent_version) = updates.agent_version {
            self.agent_version = agent_version;
        }
        if let Some(activated) = updates.activated {
            self.activated = activated;
        }
        if let Some(status) = updates.status {
            self.status = status;
        }
        if let Some(last_synced_at) = updates.last_synced_at {
            self.last_synced_at = last_synced_at;
        }
        if let Some(last_connected_at) = updates.last_connected_at {
            self.last_connected_at = last_connected_at;
        }
        if let Some(last_disconnected_at) = updates.last_disconnected_at {
            self.last_disconnected_at = last_disconnected_at;
        }
    }
}

#[derive(Debug)]
pub struct Updates {
    pub id: Option<String>,
    pub name: Option<String>,
    pub agent_version: Option<String>,
    pub activated: Option<bool>,
    pub status: Option<DeviceStatus>,
    pub last_synced_at: Option<DateTime<Utc>>,
    pub last_connected_at: Option<DateTime<Utc>>,
    pub last_disconnected_at: Option<DateTime<Utc>>,
}

impl Updates {
    pub fn empty() -> Self {
        Self {
            id: None,
            name: None,
            agent_version: None,
            activated: None,
            status: None,
            last_synced_at: None,
            last_connected_at: None,
            last_disconnected_at: None,
        }
    }

    pub fn disconnected() -> Self {
        Self {
            status: Some(DeviceStatus::Offline),
            last_disconnected_at: Some(Utc::now()),
            ..Self::empty()
        }
    }

    pub fn connected() -> Self {
        Self {
            status: Some(DeviceStatus::Online),
            last_connected_at: Some(Utc::now()),
            ..Self::empty()
        }
    }

    pub fn set_agent_version(version: String) -> Self {
        Self {
            agent_version: Some(version),
            ..Self::empty()
        }
    }
}
