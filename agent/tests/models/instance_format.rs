// internal crates
use backend_api::models as backend_client;
use miru_agent::models::InstanceFormat;

// external crates
use serde_json::json;

struct TestCase {
    format: InstanceFormat,
    wire: &'static str,
    backend: backend_client::InstanceFormat,
}

/// Every format with its wire string and backend twin, written out by hand so
/// a slip in the `impl_status_enum!` mapping table fails here.
fn cases() -> Vec<TestCase> {
    vec![
        TestCase {
            format: InstanceFormat::Json,
            wire: "json",
            backend: backend_client::InstanceFormat::INSTANCE_FORMAT_JSON,
        },
        TestCase {
            format: InstanceFormat::Yaml,
            wire: "yaml",
            backend: backend_client::InstanceFormat::INSTANCE_FORMAT_YAML,
        },
        TestCase {
            format: InstanceFormat::Jsonc,
            wire: "jsonc",
            backend: backend_client::InstanceFormat::INSTANCE_FORMAT_JSONC,
        },
        TestCase {
            format: InstanceFormat::Xml,
            wire: "xml",
            backend: backend_client::InstanceFormat::INSTANCE_FORMAT_XML,
        },
        TestCase {
            format: InstanceFormat::Text,
            wire: "text",
            backend: backend_client::InstanceFormat::INSTANCE_FORMAT_TEXT,
        },
    ]
}

#[test]
fn covers_every_variant() {
    let tested: Vec<InstanceFormat> = cases().into_iter().map(|c| c.format).collect();

    assert_eq!(tested, InstanceFormat::variants());
}

#[test]
fn wire_strings() {
    for case in cases() {
        assert_eq!(json!(case.format), json!(case.wire), "{:?}", case.format);
        let parsed: InstanceFormat = serde_json::from_value(json!(case.wire)).unwrap();
        assert_eq!(parsed, case.format);
        assert_eq!(case.format.as_str(), case.wire);
    }
}

#[test]
fn backend_conversions() {
    for case in cases() {
        assert_eq!(InstanceFormat::from(&case.backend), case.format);
        assert_eq!(
            backend_client::InstanceFormat::from(&case.format),
            case.backend
        );
    }
}

#[test]
fn backend_wire_strings_match() {
    // the agent's wire strings are the backend's, so a report's format
    // reaches Miru unchanged
    for case in cases() {
        assert_eq!(json!(case.backend), json!(case.wire), "{:?}", case.format);
    }
}

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
