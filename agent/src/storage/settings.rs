// internal crates
use crate::deserialize_warn;
use crate::logs::LogLevel;

// external crates
use serde::{Deserialize, Serialize};
use tracing::error;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Settings {
    pub log_level: LogLevel,
    pub backend: Backend,
    pub mqtt_broker: MQTTBroker,
    pub is_persistent: bool,
    pub enable_socket_server: bool,
    pub enable_mqtt_worker: bool,
    pub enable_poller: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            log_level: LogLevel::Info,
            backend: Backend::default(),
            mqtt_broker: MQTTBroker::default(),
            is_persistent: true,
            enable_socket_server: true,
            enable_mqtt_worker: true,
            enable_poller: true,
        }
    }
}

impl<'de> Deserialize<'de> for Settings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct DeserializeSettings {
            log_level: Option<LogLevel>,
            backend: Option<Backend>,
            mqtt_broker: Option<MQTTBroker>,
            is_persistent: Option<bool>,
            enable_socket_server: Option<bool>,
            enable_mqtt_worker: Option<bool>,
            enable_poller: Option<bool>,
        }

        let default = Settings::default();

        let result = match DeserializeSettings::deserialize(deserializer) {
            Ok(settings) => settings,
            Err(e) => {
                error!("Error deserializing settings: {}", e);
                return Err(e);
            }
        };

        Ok(Settings {
            log_level: result
                .log_level
                .unwrap_or_else(|| deserialize_warn!("settings", "log_level", default.log_level)),
            backend: result
                .backend
                .unwrap_or_else(|| deserialize_warn!("settings", "backend", default.backend)),
            mqtt_broker: result.mqtt_broker.unwrap_or_else(|| {
                deserialize_warn!("settings", "mqtt_broker", default.mqtt_broker)
            }),
            is_persistent: result.is_persistent.unwrap_or_else(|| {
                deserialize_warn!("settings", "is_persistent", default.is_persistent)
            }),
            enable_socket_server: result.enable_socket_server.unwrap_or_else(|| {
                deserialize_warn!(
                    "settings",
                    "enable_socket_server",
                    default.enable_socket_server
                )
            }),
            enable_mqtt_worker: result.enable_mqtt_worker.unwrap_or_else(|| {
                deserialize_warn!("settings", "enable_mqtt_worker", default.enable_mqtt_worker)
            }),
            enable_poller: result.enable_poller.unwrap_or_else(|| {
                deserialize_warn!("settings", "enable_poller", default.enable_poller)
            }),
        })
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Backend {
    pub base_url: String,
}

impl Default for Backend {
    fn default() -> Self {
        Self {
            base_url: "https://api.mirurobotics.com/agent/v1".to_string(),
        }
    }
}

impl<'de> Deserialize<'de> for Backend {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct DeserializeBackend {
            base_url: Option<String>,
        }

        let default = Backend::default();

        let result = match DeserializeBackend::deserialize(deserializer) {
            Ok(backend) => backend,
            Err(e) => {
                error!("Error deserializing backend: {}", e);
                return Err(e);
            }
        };

        Ok(Backend {
            base_url: result
                .base_url
                .unwrap_or_else(|| deserialize_warn!("backend", "base_url", default.base_url)),
        })
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct MQTTBroker {
    pub host: String,
}

impl Default for MQTTBroker {
    fn default() -> Self {
        Self {
            host: "mqtt.mirurobotics.com".to_string(),
        }
    }
}

impl<'de> Deserialize<'de> for MQTTBroker {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct DeserializeMQTTBroker {
            host: Option<String>,
        }

        let default = MQTTBroker::default();

        let result = match DeserializeMQTTBroker::deserialize(deserializer) {
            Ok(mqtt_broker) => mqtt_broker,
            Err(e) => {
                error!("Error deserializing mqtt broker: {}", e);
                return Err(e);
            }
        };

        Ok(MQTTBroker {
            host: result
                .host
                .unwrap_or_else(|| deserialize_warn!("mqtt_broker", "host", default.host)),
        })
    }
}
