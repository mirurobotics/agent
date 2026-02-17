// internal crates
use miru_agent::models::config_instance::ConfigInstance;
use openapi_client::models as backend_client;

// external crates
use chrono::{DateTime, Utc};
use serde_json::json;

// harness
use crate::models::harnesses::{serde_tests, ModelFixture, OptionalField, RequiredField};

// ─── fixture ─────────────────────────────────────────────────────────────────

impl ModelFixture for ConfigInstance {
    fn required_fields() -> Vec<RequiredField> {
        vec![
            RequiredField {
                key: "id",
                value: json!("cfg_inst_123"),
            },
            RequiredField {
                key: "config_type_name",
                value: json!("motion-control"),
            },
            RequiredField {
                key: "filepath",
                value: json!("v1/motion-control.json"),
            },
            RequiredField {
                key: "config_schema_id",
                value: json!("schema_123"),
            },
            RequiredField {
                key: "config_type_id",
                value: json!("type_123"),
            },
        ]
    }

    fn optional_fields() -> Vec<OptionalField> {
        vec![OptionalField {
            key: "created_at",
            value: json!("2023-11-14T22:13:20Z"),
            default_value: json!("1970-01-01T00:00:00Z"),
        }]
    }
}

serde_tests!(ConfigInstance);

#[test]
fn defaults() {
    let instance = ConfigInstance::default();

    let id = instance.id.clone();
    assert!(id.starts_with("unknown-"));
    let filepath = instance.filepath.clone();
    assert!(filepath.starts_with("unknown-"));
    let config_schema_id = instance.config_schema_id.clone();
    assert!(config_schema_id.starts_with("unknown-"));
    let config_type_id = instance.config_type_id.clone();
    assert!(config_type_id.starts_with("unknown-"));

    let expected = ConfigInstance {
        id,
        config_type_name: String::new(),
        filepath,
        created_at: DateTime::<Utc>::UNIX_EPOCH,
        config_schema_id,
        config_type_id,
    };
    assert_eq!(instance, expected);
}

// ─── from-backend tests ────────────────────────────────────────────────────

#[test]
fn from_backend() {
    let now = Utc::now();
    let backend_instance = backend_client::ConfigInstance {
        object: backend_client::config_instance::Object::ConfigInstance,
        id: "cfg_inst_123".to_string(),
        config_type_name: "motion-control".to_string(),
        filepath: "v1/motion-control.json".to_string(),
        created_at: now.to_rfc3339(),
        config_schema_id: "schema_123".to_string(),
        config_type_id: "type_123".to_string(),
        config_type: None,
        content: None,
    };

    let actual = ConfigInstance::from_backend(backend_instance);
    let expected = ConfigInstance {
        id: "cfg_inst_123".to_string(),
        config_type_name: "motion-control".to_string(),
        filepath: "v1/motion-control.json".to_string(),
        config_schema_id: "schema_123".to_string(),
        config_type_id: "type_123".to_string(),
        created_at: now,
    };
    assert_eq!(actual, expected);
}

#[test]
fn from_backend_invalid_date() {
    let backend_instance = backend_client::ConfigInstance {
        object: backend_client::config_instance::Object::ConfigInstance,
        id: "cfg_inst_789".to_string(),
        config_type_name: "test-type".to_string(),
        filepath: "v1/test.json".to_string(),
        created_at: "not-a-date".to_string(),
        config_schema_id: "schema_789".to_string(),
        config_type_id: "type_789".to_string(),
        config_type: None,
        content: None,
    };

    let instance = ConfigInstance::from_backend(backend_instance);
    assert_eq!(instance.id, "cfg_inst_789");
    assert_eq!(instance.created_at, DateTime::<Utc>::UNIX_EPOCH);
}
