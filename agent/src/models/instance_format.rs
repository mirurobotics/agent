// internal crates
use crate::models::status::impl_status_enum;
use backend_api::models as backend_client;
use device_api::models as agent_server;

// external crates
use serde::Serialize;
use tracing::warn;

/// The serialization format of instance content, such as a calibration's data.
#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum InstanceFormat {
    Json,
    Yaml,
    Jsonc,
    Xml,
    /// Plain text, and the fallback for a format this agent doesn't know.
    #[default]
    Text,
}

impl_status_enum!(
    enum InstanceFormat,
    default: Text,
    label: "instance format",
    log: warn,
    backend_type: backend_client::InstanceFormat,
    unknown_backend: backend_client::InstanceFormat::InstanceFormatUnknown,
    mappings: [
        Json => "json" => backend_client::InstanceFormat::INSTANCE_FORMAT_JSON,
        Yaml => "yaml" => backend_client::InstanceFormat::INSTANCE_FORMAT_YAML,
        Jsonc => "jsonc" => backend_client::InstanceFormat::INSTANCE_FORMAT_JSONC,
        Xml => "xml" => backend_client::InstanceFormat::INSTANCE_FORMAT_XML,
        Text => "text" => backend_client::InstanceFormat::INSTANCE_FORMAT_TEXT,
    ]
);

impl InstanceFormat {
    /// The format a Device API request names, or `None` for one this agent
    /// doesn't know: a request with it is rejected, not defaulted to text.
    pub fn from_device_api(format: agent_server::InstanceFormat) -> Option<Self> {
        match format {
            agent_server::InstanceFormat::INSTANCE_FORMAT_JSON => Some(Self::Json),
            agent_server::InstanceFormat::INSTANCE_FORMAT_YAML => Some(Self::Yaml),
            agent_server::InstanceFormat::INSTANCE_FORMAT_JSONC => Some(Self::Jsonc),
            agent_server::InstanceFormat::INSTANCE_FORMAT_XML => Some(Self::Xml),
            agent_server::InstanceFormat::INSTANCE_FORMAT_TEXT => Some(Self::Text),
            agent_server::InstanceFormat::InstanceFormatUnknown => None,
        }
    }
}
