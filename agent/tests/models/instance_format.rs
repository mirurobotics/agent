// internal crates
use backend_api::models as backend_client;
use device_api::models as agent_server;
use miru_agent::models::InstanceFormat;

// harness
use crate::models::harnesses::{status_enum_tests, StatusCase, StatusEnumFixture};

// external crates
use serde_json::json;

impl StatusEnumFixture for InstanceFormat {
    type Backend = backend_client::InstanceFormat;
    type Agent = ();

    fn cases() -> Vec<StatusCase<Self>> {
        vec![
            StatusCase {
                variant: InstanceFormat::Json,
                wire: "json",
                backend: backend_client::InstanceFormat::INSTANCE_FORMAT_JSON,
                agent: (),
            },
            StatusCase {
                variant: InstanceFormat::Yaml,
                wire: "yaml",
                backend: backend_client::InstanceFormat::INSTANCE_FORMAT_YAML,
                agent: (),
            },
            StatusCase {
                variant: InstanceFormat::Jsonc,
                wire: "jsonc",
                backend: backend_client::InstanceFormat::INSTANCE_FORMAT_JSONC,
                agent: (),
            },
            StatusCase {
                variant: InstanceFormat::Xml,
                wire: "xml",
                backend: backend_client::InstanceFormat::INSTANCE_FORMAT_XML,
                agent: (),
            },
            StatusCase {
                variant: InstanceFormat::Text,
                wire: "text",
                backend: backend_client::InstanceFormat::INSTANCE_FORMAT_TEXT,
                agent: (),
            },
        ]
    }
}

status_enum_tests!(InstanceFormat, backend);

#[test]
fn defaults_to_text() {
    assert_eq!(InstanceFormat::default(), InstanceFormat::Text);
}

#[test]
fn unknown_falls_back_to_text() {
    let parsed: InstanceFormat = serde_json::from_value(json!("toml")).unwrap();
    let unknown = backend_client::InstanceFormat::InstanceFormatUnknown;

    assert_eq!(parsed, InstanceFormat::Text);
    assert_eq!(InstanceFormat::from(&unknown), InstanceFormat::Text);
}

#[test]
fn from_device_api_maps_each_known_format() {
    let cases = [
        (
            agent_server::InstanceFormat::INSTANCE_FORMAT_JSON,
            InstanceFormat::Json,
        ),
        (
            agent_server::InstanceFormat::INSTANCE_FORMAT_YAML,
            InstanceFormat::Yaml,
        ),
        (
            agent_server::InstanceFormat::INSTANCE_FORMAT_JSONC,
            InstanceFormat::Jsonc,
        ),
        (
            agent_server::InstanceFormat::INSTANCE_FORMAT_XML,
            InstanceFormat::Xml,
        ),
        (
            agent_server::InstanceFormat::INSTANCE_FORMAT_TEXT,
            InstanceFormat::Text,
        ),
    ];
    for (format, want) in cases {
        assert_eq!(
            InstanceFormat::from_device_api(format),
            Some(want),
            "{format:?}"
        );
    }
}

#[test]
fn from_device_api_rejects_an_unknown_format() {
    let unknown = agent_server::InstanceFormat::InstanceFormatUnknown;
    assert_eq!(InstanceFormat::from_device_api(unknown), None);
}
