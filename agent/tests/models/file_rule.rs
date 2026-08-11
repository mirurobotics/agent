// internal crates
use backend_api::models as backend_client;
use miru_agent::models::{FileRule, FileRuleRetention, FileRuleSource, FileRuleUpload};

// external crates
use chrono::{DateTime, Utc};
use serde_json::json;

// harness
use crate::models::harnesses::{serde_tests, ModelFixture, OptionalField, RequiredField};

// ─── fixture ─────────────────────────────────────────────────────────────────

impl ModelFixture for FileRule {
    fn required_fields() -> Vec<RequiredField> {
        vec![
            RequiredField {
                key: "id",
                value: json!("upl_rule_123"),
            },
            RequiredField {
                key: "name",
                value: json!("camera-logs"),
            },
            RequiredField {
                key: "digest",
                value: json!("sha256:abc123"),
            },
            RequiredField {
                key: "source",
                value: json!({
                    "glob": "/data/*.mcap",
                    "stability_window_secs": 30,
                }),
            },
        ]
    }

    fn optional_fields() -> Vec<OptionalField> {
        vec![
            OptionalField {
                key: "upload",
                value: json!({
                    "upload_collection_id": "upl_col_123",
                    "upload_collection_name": "camera-logs",
                    "bucket_id": "buck_123",
                    "bucket_name": "my-bucket",
                    "path": "{device_id}/logs",
                }),
                default_value: json!(null),
            },
            OptionalField {
                key: "retention",
                value: json!({
                    "require_upload": true,
                    "ttl_secs": 0,
                }),
                default_value: json!(null),
            },
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

serde_tests!(FileRule);

#[test]
fn defaults() {
    let actual = FileRule::default();

    let id = actual.id.clone();
    assert!(id.starts_with("unknown-"));

    let expected = FileRule {
        id,
        name: String::new(),
        digest: String::new(),
        source: FileRuleSource::default(),
        upload: None,
        retention: None,
        created_at: DateTime::<Utc>::UNIX_EPOCH,
        updated_at: DateTime::<Utc>::UNIX_EPOCH,
    };
    assert_eq!(actual, expected);
}

// ─── from-backend tests ────────────────────────────────────────────────────

fn backend_rule(
    created_at: &str,
    updated_at: &str,
    delete_policy: backend_client::UploadDeletePolicy,
) -> backend_client::BaseUploadRule {
    backend_client::BaseUploadRule {
        object: backend_client::base_upload_rule::Object::UploadRule,
        id: "upl_rule_123".to_string(),
        upload_collection_id: "upl_col_123".to_string(),
        upload_collection_name: "camera-logs".to_string(),
        digest: "sha256:abc123".to_string(),
        source: Box::new(backend_client::UploadRuleSource {
            glob: "/data/*.mcap".to_string(),
            stability_window_secs: 30,
        }),
        destination: Box::new(backend_client::UploadRuleDestination {
            bucket_id: "buck_123".to_string(),
            bucket_name: "my-bucket".to_string(),
            path: "{device_id}/logs".to_string(),
            delete_policy,
        }),
        created_at: created_at.to_string(),
        updated_at: updated_at.to_string(),
    }
}

#[test]
fn from_backend() {
    let created = Utc::now();
    let updated = Utc::now();
    let backend = backend_rule(
        &created.to_rfc3339(),
        &updated.to_rfc3339(),
        backend_client::UploadDeletePolicy::UPLOAD_DELETE_POLICY_AFTER_UPLOAD,
    );

    let actual: FileRule = backend.into();
    let expected = FileRule {
        id: "upl_rule_123".to_string(),
        name: "camera-logs".to_string(),
        digest: "sha256:abc123".to_string(),
        source: FileRuleSource {
            glob: "/data/*.mcap".to_string(),
            stability_window_secs: 30,
        },
        upload: Some(FileRuleUpload {
            upload_collection_id: "upl_col_123".to_string(),
            upload_collection_name: "camera-logs".to_string(),
            bucket_id: "buck_123".to_string(),
            bucket_name: "my-bucket".to_string(),
            path: "{device_id}/logs".to_string(),
        }),
        retention: Some(FileRuleRetention {
            require_upload: true,
            ttl_secs: 0,
        }),
        created_at: created,
        updated_at: updated,
    };
    assert_eq!(actual, expected); // lint:allow(field-by-field-assert)
}

// A wire rule with `delete_policy: never` maps to no retention block at all,
// which is what preserves the old "never delete" behavior.
#[test]
fn from_backend_never_delete_policy_has_no_retention() {
    let now = Utc::now();
    let backend = backend_rule(
        &now.to_rfc3339(),
        &now.to_rfc3339(),
        backend_client::UploadDeletePolicy::UPLOAD_DELETE_POLICY_NEVER,
    );

    let rule: FileRule = backend.into();
    assert_eq!(rule.retention, None);
}

// An unrecognized wire delete policy is treated as "never" rather than
// erroring, so a backend that adds a new policy cannot cause data loss.
#[test]
fn from_backend_unknown_delete_policy_has_no_retention() {
    let now = Utc::now();
    let backend = backend_rule(
        &now.to_rfc3339(),
        &now.to_rfc3339(),
        backend_client::UploadDeletePolicy::UploadDeletePolicyUnknown,
    );

    let rule: FileRule = backend.into();
    assert_eq!(rule.retention, None);
}

#[test]
fn from_backend_invalid_dates() {
    let backend = backend_rule(
        "not-a-date",
        "also-not-a-date",
        backend_client::UploadDeletePolicy::UPLOAD_DELETE_POLICY_AFTER_UPLOAD,
    );

    let rule: FileRule = backend.into();
    assert_eq!(rule.id, "upl_rule_123");
    assert_eq!(rule.created_at, DateTime::<Utc>::UNIX_EPOCH);
    assert_eq!(rule.updated_at, DateTime::<Utc>::UNIX_EPOCH);
}

// ─── retention defaults ─────────────────────────────────────────────────────

#[test]
fn retention_default() {
    assert_eq!(
        FileRuleRetention::default(),
        FileRuleRetention {
            require_upload: false,
            ttl_secs: 0,
        }
    );
}
