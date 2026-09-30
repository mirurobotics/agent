// internal crates
use crate::deserialize_warn;
use crate::logs::LogLevel;
use crate::network::{BackendHost, MqttHost};
use crate::server::{DEFAULT_ENABLE_TCP_SERVER, DEFAULT_TCP_PORT};

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
    /// Loopback TCP listener for the local device API. Independent of the Unix
    /// socket. On by default.
    pub enable_tcp_server: bool,
    pub tcp_server: TCPServer,
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
            enable_tcp_server: DEFAULT_ENABLE_TCP_SERVER,
            tcp_server: TCPServer::default(),
            enable_mqtt_worker: true,
            enable_poller: true,
        }
    }
}

#[derive(Deserialize)]
struct DeserializeSettings {
    log_level: Option<LogLevel>,
    backend: Option<Backend>,
    mqtt_broker: Option<MQTTBroker>,
    is_persistent: Option<bool>,
    enable_socket_server: Option<bool>,
    enable_tcp_server: Option<bool>,
    tcp_server: Option<TCPServer>,
    enable_mqtt_worker: Option<bool>,
    enable_poller: Option<bool>,
}

impl<'de> Deserialize<'de> for Settings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
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
            enable_tcp_server: result.enable_tcp_server.unwrap_or_else(|| {
                deserialize_warn!("settings", "enable_tcp_server", default.enable_tcp_server)
            }),
            tcp_server: result
                .tcp_server
                .unwrap_or_else(|| deserialize_warn!("settings", "tcp_server", default.tcp_server)),
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
pub struct TCPServer {
    /// Loopback TCP port. `0` lets the OS assign a port.
    pub port: u16,
}

impl Default for TCPServer {
    fn default() -> Self {
        Self {
            port: DEFAULT_TCP_PORT,
        }
    }
}

impl<'de> Deserialize<'de> for TCPServer {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct DeserializeTCPServer {
            port: Option<u16>,
        }

        let default = TCPServer::default();
        let result = match DeserializeTCPServer::deserialize(deserializer) {
            Ok(server) => server,
            Err(e) => {
                error!("Error deserializing tcp server: {}", e);
                return Err(e);
            }
        };
        Ok(TCPServer {
            port: result
                .port
                .unwrap_or_else(|| deserialize_warn!("tcp_server", "port", default.port)),
        })
    }
}

#[derive(Debug, Default, Serialize, PartialEq, Eq)]
pub struct Backend {
    pub host: BackendHost,
}

impl<'de> Deserialize<'de> for Backend {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct DeserializeBackend {
            host: Option<String>,
        }

        let default = Backend::default();

        let result = match DeserializeBackend::deserialize(deserializer) {
            Ok(backend) => backend,
            Err(e) => {
                error!("Error deserializing backend: {}", e);
                return Err(e);
            }
        };

        let raw = result.host.unwrap_or_else(|| {
            deserialize_warn!("backend", "host", default.host.as_str().to_string())
        });
        Ok(Backend {
            host: BackendHost::new_or(&raw, default.host),
        })
    }
}

#[derive(Debug, Serialize, PartialEq, Eq, Default)]
pub struct MQTTBroker {
    pub host: MqttHost,
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
                error!("error deserializing mqtt broker: {}", e);
                return Err(e);
            }
        };

        let raw = result.host.unwrap_or_else(|| {
            deserialize_warn!("mqtt_broker", "host", default.host.as_str().to_string())
        });
        let host = MqttHost::new_or(&raw, default.host);
        Ok(MQTTBroker { host })
    }
}
