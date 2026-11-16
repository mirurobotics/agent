// internal crates
use backend_api::models as backend_client;
use miru_daemon::models::InstanceFormat;

// harness
use crate::models::harnesses::{status_enum_tests, StatusCase, StatusEnumFixture};

// external crates
use serde_json::json;

impl StatusEnumFixture for InstanceFormat {
    type Backend = backend_client::InstanceFormat;

    fn cases() -> Vec<StatusCase<Self>> {
        vec![
            StatusCase {
                variant: InstanceFormat::Json,
                wire: "json",
                backend: backend_client::InstanceFormat::INSTANCE_FORMAT_JSON,
            },
            StatusCase {
                variant: InstanceFormat::Yaml,
                wire: "yaml",
                backend: backend_client::InstanceFormat::INSTANCE_FORMAT_YAML,
            },
            StatusCase {
                variant: InstanceFormat::Jsonc,
                wire: "jsonc",
                backend: backend_client::InstanceFormat::INSTANCE_FORMAT_JSONC,
            },
            StatusCase {
                variant: InstanceFormat::Xml,
                wire: "xml",
                backend: backend_client::InstanceFormat::INSTANCE_FORMAT_XML,
            },
            StatusCase {
                variant: InstanceFormat::Csv,
                wire: "csv",
                backend: backend_client::InstanceFormat::INSTANCE_FORMAT_CSV,
            },
            StatusCase {
                variant: InstanceFormat::Text,
                wire: "text",
                backend: backend_client::InstanceFormat::INSTANCE_FORMAT_TEXT,
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
fn media_type_matches_mirus_content_downloads() {
    let cases = [
        (InstanceFormat::Json, "application/json"),
        (InstanceFormat::Yaml, "application/yaml"),
        (InstanceFormat::Jsonc, "application/jsonc"),
        (InstanceFormat::Xml, "application/xml"),
        (InstanceFormat::Csv, "text/csv"),
        (InstanceFormat::Text, "application/octet-stream"),
    ];
    for (format, want) in cases {
        assert_eq!(format.media_type(), want, "{format:?}");
    }
}

#[test]
fn file_ext_matches_mirus_file_names() {
    let cases = [
        (InstanceFormat::Json, Some("json")),
        (InstanceFormat::Yaml, Some("yaml")),
        (InstanceFormat::Jsonc, Some("jsonc")),
        (InstanceFormat::Xml, Some("xml")),
        (InstanceFormat::Csv, Some("csv")),
        (InstanceFormat::Text, None),
    ];
    for (format, want) in cases {
        assert_eq!(format.file_ext(), want, "{format:?}");
    }
}
