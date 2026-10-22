// internal crates
use crate::models::status::impl_status_enum;
use backend_api::models as backend_client;

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
