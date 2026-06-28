// internal crates
use backend_api::models as backend_client;
use miru_agent::models::{DeletePolicy, UploadRule, UploadRuleDestination, UploadRuleSource};

// external crates
use chrono::{DateTime, Utc};
use serde_json::json;

// harness
use crate::models::harnesses::{
    serde_tests, status_serde_tests, ModelFixture, OptionalField, RequiredField, StatusCase,
    StatusFixture,
};

// ─── fixture ─────────────────────────────────────────────────────────────────

impl ModelFixture for UploadRule {
    fn required_fields() -> Vec<RequiredField> {
        vec![
            RequiredField {
                key: "id",
                value: json!("upl_rule_123"),
            },
            RequiredField {
                key: "upload_collection_id",
                value: json!("upl_col_123"),
            },
            RequiredField {
                key: "upload_collection_name",
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
                    "poll_interval_secs": 60,
                    "stability_window_secs": 30,
                }),
            },
            RequiredField {
                key: "destination",
                value: json!({
                    "bucket_id": "buck_123",
                    "bucket_name": "my-bucket",
                    "path": "{device_id}/logs",
                    "delete_policy": "never",
                }),
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

serde_tests!(UploadRule);

#[test]
fn defaults() {
    let actual = UploadRule::default();

    let id = actual.id.clone();
    assert!(id.starts_with("unknown-"));
    let upload_collection_id = actual.upload_collection_id.clone();
    assert!(upload_collection_id.starts_with("unknown-"));

    let expected = UploadRule {
        id,
        upload_collection_id,
        upload_collection_name: String::new(),
        digest: String::new(),
        source: UploadRuleSource::default(),
        destination: UploadRuleDestination::default(),
        created_at: DateTime::<Utc>::UNIX_EPOCH,
        updated_at: DateTime::<Utc>::UNIX_EPOCH,
    };
    assert_eq!(actual, expected);
}

// ─── from-backend tests ────────────────────────────────────────────────────

fn backend_rule(created_at: &str, updated_at: &str) -> backend_client::BaseUploadRule {
    backend_client::BaseUploadRule {
        object: backend_client::base_upload_rule::Object::UploadRule,
        id: "upl_rule_123".to_string(),
        upload_collection_id: "upl_col_123".to_string(),
        upload_collection_name: "camera-logs".to_string(),
        digest: "sha256:abc123".to_string(),
        source: Box::new(backend_client::UploadRuleSource {
            glob: "/data/*.mcap".to_string(),
            poll_interval_secs: 60,
            stability_window_secs: 30,
        }),
        destination: Box::new(backend_client::UploadRuleDestination {
            bucket_id: "buck_123".to_string(),
            bucket_name: "my-bucket".to_string(),
            path: "{device_id}/logs".to_string(),
            delete_policy: backend_client::UploadDeletePolicy::UPLOAD_DELETE_POLICY_AFTER_UPLOAD,
        }),
        created_at: created_at.to_string(),
        updated_at: updated_at.to_string(),
    }
}

#[test]
fn from_backend() {
    let created = Utc::now();
    let updated = Utc::now();
    let backend = backend_rule(&created.to_rfc3339(), &updated.to_rfc3339());

    let actual: UploadRule = backend.into();
    let expected = UploadRule {
        id: "upl_rule_123".to_string(),
        upload_collection_id: "upl_col_123".to_string(),
        upload_collection_name: "camera-logs".to_string(),
        digest: "sha256:abc123".to_string(),
        source: UploadRuleSource {
            glob: "/data/*.mcap".to_string(),
            poll_interval_secs: 60,
            stability_window_secs: 30,
        },
        destination: UploadRuleDestination {
            bucket_id: "buck_123".to_string(),
            bucket_name: "my-bucket".to_string(),
            path: "{device_id}/logs".to_string(),
            delete_policy: DeletePolicy::AfterUpload,
        },
        created_at: created,
        updated_at: updated,
    };
    assert_eq!(actual, expected); // lint:allow(field-by-field-assert)
}

#[test]
fn from_backend_invalid_dates() {
    let backend = backend_rule("not-a-date", "also-not-a-date");

    let rule: UploadRule = backend.into();
    assert_eq!(rule.id, "upl_rule_123");
    assert_eq!(rule.created_at, DateTime::<Utc>::UNIX_EPOCH);
    assert_eq!(rule.updated_at, DateTime::<Utc>::UNIX_EPOCH);
}

// ─── delete-policy status enum tests ──────────────────────────────────────────

// DeletePolicy is a backend-only enum (hand-rolled, not via `impl_status_enum!`),
// so it has no generated `variants()`/`as_str()`; spell those out here. The
// serde suite comes from the shared `status_serde_tests!` harness; the
// backend->domain conversion and `Display` are not covered by the harness and
// keep their explicit tests below.
impl StatusFixture for DeletePolicy {
    fn variants() -> Vec<Self> {
        vec![DeletePolicy::Never, DeletePolicy::AfterUpload]
    }
    fn wire_str(&self) -> &'static str {
        match self {
            DeletePolicy::Never => "never",
            DeletePolicy::AfterUpload => "after_upload",
        }
    }
    fn cases() -> Vec<StatusCase<Self>> {
        vec![
            StatusCase {
                input: "\"never\"",
                expected: DeletePolicy::Never,
                valid: true,
            },
            StatusCase {
                input: "\"after_upload\"",
                expected: DeletePolicy::AfterUpload,
                valid: true,
            },
            StatusCase {
                input: "\"unknown\"",
                expected: DeletePolicy::Never,
                valid: false,
            },
        ]
    }
}

mod delete_policy {
    use super::*;

    status_serde_tests!(DeletePolicy);

    #[test]
    fn display() {
        assert_eq!(DeletePolicy::Never.to_string(), "never");
        assert_eq!(DeletePolicy::AfterUpload.to_string(), "after_upload");
    }

    #[test]
    fn from_backend_known() {
        let never: DeletePolicy =
            (&backend_client::UploadDeletePolicy::UPLOAD_DELETE_POLICY_NEVER).into();
        assert_eq!(never, DeletePolicy::Never);

        let after: DeletePolicy =
            (&backend_client::UploadDeletePolicy::UPLOAD_DELETE_POLICY_AFTER_UPLOAD).into();
        assert_eq!(after, DeletePolicy::AfterUpload);
    }

    #[test]
    fn from_backend_unknown_defaults_to_never() {
        let unknown: DeletePolicy =
            (&backend_client::UploadDeletePolicy::UploadDeletePolicyUnknown).into();
        assert_eq!(unknown, DeletePolicy::Never);
    }
}

#[test]
fn delete_policy_default() {
    assert_eq!(DeletePolicy::default(), DeletePolicy::Never);
}
