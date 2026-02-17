// internal crates
use miru_agent::models::release::Release;
use openapi_client::models::Release as BackendRelease;

// external crates
use chrono::{DateTime, Utc};
use serde_json::json;

// harness
use crate::models::harnesses::{serde_tests, ModelFixture, OptionalField, RequiredField};

// ─── fixture ─────────────────────────────────────────────────────────────────

impl ModelFixture for Release {
    fn required_fields() -> Vec<RequiredField> {
        vec![
            RequiredField {
                key: "id",
                value: json!("rel_123"),
            },
            RequiredField {
                key: "version",
                value: json!("1.0.0"),
            },
        ]
    }

    fn optional_fields() -> Vec<OptionalField> {
        vec![
            OptionalField {
                key: "created_at",
                value: json!("2023-11-14T22:13:20Z"),
                default_value: json!("1970-01-01T00:00:00Z"),
            },
            OptionalField {
                key: "updated_at",
                value: json!("2023-11-14T22:15:00Z"),
                default_value: json!("1970-01-01T00:00:00Z"),
            },
        ]
    }
}

serde_tests!(Release);

#[test]
fn defaults() {
    let actual = Release::default();
    let id = actual.id.clone();
    assert!(id.starts_with("unknown-"));
    let expected = Release {
        id,
        version: String::new(),
        created_at: DateTime::<Utc>::UNIX_EPOCH,
        updated_at: DateTime::<Utc>::UNIX_EPOCH,
    };
    assert_eq!(actual, expected);
}

// ─── model-specific tests ────────────────────────────────────────────────────

#[test]
fn from_backend() {
    let now = Utc::now();
    let backend_release = BackendRelease {
        object: openapi_client::models::release::Object::Release,
        id: "rel_123".to_string(),
        version: "1.0.0".to_string(),
        created_at: now.to_rfc3339(),
        updated_at: now.to_rfc3339(),
    };

    let release = Release::from_backend(backend_release);

    assert_eq!(release.id, "rel_123");
    assert_eq!(release.version, "1.0.0");
    assert!(release.created_at > DateTime::<Utc>::UNIX_EPOCH);
    assert!(release.updated_at > DateTime::<Utc>::UNIX_EPOCH);
}

#[test]
fn from_backend_invalid_dates() {
    let backend_release = BackendRelease {
        object: openapi_client::models::release::Object::Release,
        id: "rel_789".to_string(),
        version: "3.0.0".to_string(),
        created_at: "not-a-date".to_string(),
        updated_at: "also-not-a-date".to_string(),
    };

    let release = Release::from_backend(backend_release);
    assert_eq!(release.id, "rel_789");
    assert_eq!(release.created_at, DateTime::<Utc>::UNIX_EPOCH);
    assert_eq!(release.updated_at, DateTime::<Utc>::UNIX_EPOCH);
}
