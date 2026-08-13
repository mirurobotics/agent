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

fn backend_upload() -> backend_client::FileRuleUpload {
    backend_client::FileRuleUpload {
        upload_collection_id: "upl_col_123".to_string(),
        upload_collection_name: "camera-logs".to_string(),
        bucket_id: "buck_123".to_string(),
        bucket_name: "my-bucket".to_string(),
        path: "{device_id}/logs".to_string(),
    }
}

fn backend_rule(
    created_at: &str,
    updated_at: &str,
    upload: Option<backend_client::FileRuleUpload>,
    retention: Option<backend_client::FileRuleRetention>,
) -> backend_client::BaseFileRule {
    backend_client::BaseFileRule {
        object: backend_client::base_file_rule::Object::FileRule,
        id: "file_rule_123".to_string(),
        name: "camera-logs".to_string(),
        digest: "sha256:abc123".to_string(),
        source: Box::new(backend_client::FileRuleSource {
            glob: "/data/*.mcap".to_string(),
            stability_window_secs: 30,
        }),
        upload: upload.map(Box::new),
        retention: retention.map(Box::new),
        created_at: created_at.to_string(),
        updated_at: updated_at.to_string(),
    }
}

// A rule that both uploads and deletes: every wire field must land on the
// domain type, including all five upload fields and both retention fields.
#[test]
fn from_backend() {
    let created = Utc::now();
    let updated = Utc::now();
    let backend = backend_rule(
        &created.to_rfc3339(),
        &updated.to_rfc3339(),
        Some(backend_upload()),
        Some(backend_client::FileRuleRetention {
            require_upload: Some(true),
            ttl_secs: 3600,
        }),
    );

    let actual: FileRule = backend.into();
    let expected = FileRule {
        id: "file_rule_123".to_string(),
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
            ttl_secs: 3600,
        }),
        created_at: created,
        updated_at: updated,
    };
    assert_eq!(actual, expected); // lint:allow(field-by-field-assert)
}

// A bare rule carries neither an upload block nor a retention block: both
// optional domain fields stay `None` while the rest still maps.
#[test]
fn from_backend_no_upload_or_retention() {
    let created = Utc::now();
    let updated = Utc::now();
    let backend = backend_rule(&created.to_rfc3339(), &updated.to_rfc3339(), None, None);

    let actual: FileRule = backend.into();
    let expected = FileRule {
        id: "file_rule_123".to_string(),
        name: "camera-logs".to_string(),
        digest: "sha256:abc123".to_string(),
        source: FileRuleSource {
            glob: "/data/*.mcap".to_string(),
            stability_window_secs: 30,
        },
        upload: None,
        retention: None,
        created_at: created,
        updated_at: updated,
    };
    assert_eq!(actual, expected); // lint:allow(field-by-field-assert)
}

// The spec marks `require_upload` optional: it is present exactly when the rule
// has an upload block, so an absent value means there is nothing to wait on.
#[test]
fn from_backend_absent_require_upload_is_false() {
    let now = Utc::now().to_rfc3339();
    let backend = backend_rule(
        &now,
        &now,
        None,
        Some(backend_client::FileRuleRetention {
            require_upload: None,
            ttl_secs: 60,
        }),
    );

    let rule: FileRule = backend.into();
    assert_eq!(
        rule.retention,
        Some(FileRuleRetention {
            require_upload: false,
            ttl_secs: 60,
        })
    );
}

// The wire types `ttl_secs` as a signed int64 while the domain uses u64: a
// negative wire value clamps to zero rather than wrapping around.
#[test]
fn from_backend_negative_ttl_secs_clamps_to_zero() {
    let now = Utc::now().to_rfc3339();
    let backend = backend_rule(
        &now,
        &now,
        Some(backend_upload()),
        Some(backend_client::FileRuleRetention {
            require_upload: Some(true),
            ttl_secs: -1,
        }),
    );

    let rule: FileRule = backend.into();
    assert_eq!(
        rule.retention,
        Some(FileRuleRetention {
            require_upload: true,
            ttl_secs: 0,
        })
    );
}

#[test]
fn from_backend_zero_ttl_secs_is_preserved() {
    let now = Utc::now().to_rfc3339();
    let backend = backend_rule(
        &now,
        &now,
        Some(backend_upload()),
        Some(backend_client::FileRuleRetention {
            require_upload: Some(false),
            ttl_secs: 0,
        }),
    );

    let rule: FileRule = backend.into();
    assert_eq!(
        rule.retention,
        Some(FileRuleRetention {
            require_upload: false,
            ttl_secs: 0,
        })
    );
}

#[test]
fn from_backend_invalid_dates() {
    let backend = backend_rule("not-a-date", "also-not-a-date", None, None);

    let rule: FileRule = backend.into();
    assert_eq!(rule.id, "file_rule_123");
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
