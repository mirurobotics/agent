// internal crates
use crate::deserialize_error;
use crate::models::{status::impl_status_enum, Patch};
use device_api::models as agent_server;

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

impl_status_enum!(
    enum DeviceStatus,
    default: Offline,
    label: "device status",
    log: error,
    agent_type: agent_server::DeviceStatus,
    mappings: [
        Online => "online" =>
            agent_server::DeviceStatus::DEVICE_STATUS_ONLINE,
        Offline => "offline" =>
            agent_server::DeviceStatus::DEVICE_STATUS_OFFLINE,
    ]
);

#[derive(Debug, Serialize, PartialEq, Eq, Clone)]
pub struct Device {
    #[serde(rename = "device_id")]
    pub id: String,
    pub session_id: String,
    pub name: String,
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

impl From<&backend_api::models::Device> for Device {
    fn from(api_device: &backend_api::models::Device) -> Device {
        Device {
            id: api_device.id.clone(),
            name: api_device.name.clone(),
            session_id: api_device.session_id.clone(),
            activated: true,
            status: DeviceStatus::Online,
            last_synced_at: DateTime::<Utc>::UNIX_EPOCH,
            last_connected_at: DateTime::<Utc>::UNIX_EPOCH,
            last_disconnected_at: DateTime::<Utc>::UNIX_EPOCH,
        }
    }
}

impl Patch<Updates> for Device {
    fn patch(&mut self, patch: Updates) {
        if let Some(id) = patch.id {
            self.id = id;
        }
        if let Some(name) = patch.name {
            self.name = name;
        }
        if let Some(activated) = patch.activated {
            self.activated = activated;
        }
        if let Some(status) = patch.status {
            self.status = status;
        }
        if let Some(last_synced_at) = patch.last_synced_at {
            self.last_synced_at = last_synced_at;
        }
        if let Some(last_connected_at) = patch.last_connected_at {
            self.last_connected_at = last_connected_at;
        }
        if let Some(last_disconnected_at) = patch.last_disconnected_at {
            self.last_disconnected_at = last_disconnected_at;
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Updates {
    pub id: Option<String>,
    pub name: Option<String>,
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

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_openapi_device_maps_fields() {
        let api_device = backend_api::models::Device {
            id: "dev-123".to_string(),
            name: "my-robot".to_string(),
            session_id: "sess-456".to_string(),
            ..Default::default()
        };

        let device: Device = (&api_device).into();

        assert_eq!(device.id, "dev-123");
        assert_eq!(device.name, "my-robot");
        assert_eq!(device.session_id, "sess-456");
        assert!(device.activated);
        assert_eq!(device.status, DeviceStatus::Online);
        assert_eq!(device.last_synced_at, DateTime::<Utc>::UNIX_EPOCH);
        assert_eq!(device.last_connected_at, DateTime::<Utc>::UNIX_EPOCH);
        assert_eq!(device.last_disconnected_at, DateTime::<Utc>::UNIX_EPOCH);
    }
}
