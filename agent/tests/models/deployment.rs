// internal crates
use miru_agent::models::deployment::{
    Deployment, DeploymentActivityStatus, DeploymentErrorStatus, DeploymentStatus,
    DeploymentTargetStatus,
};
use openapi_client::models as backend_client;

// external crates
use chrono::{DateTime, TimeDelta, Utc};
use serde_json::json;

// harness
use crate::models::harnesses::{
    serde_tests, status_serde_tests, ModelFixture, OptionalField, RequiredField, StatusCase,
    StatusFixture,
};

// ─── model tests ───────────────────────────────────────────────────────────

impl ModelFixture for Deployment {
    fn required_fields() -> Vec<RequiredField> {
        vec![
            RequiredField {
                key: "id",
                value: json!("dpl_123"),
            },
            RequiredField {
                key: "description",
                value: json!("Test"),
            },
            RequiredField {
                key: "status",
                value: json!("deployed"),
            },
            RequiredField {
                key: "activity_status",
                value: json!("deployed"),
            },
            RequiredField {
                key: "error_status",
                value: json!("none"),
            },
            RequiredField {
                key: "target_status",
                value: json!("deployed"),
            },
            RequiredField {
                key: "device_id",
                value: json!("device_123"),
            },
            RequiredField {
                key: "release_id",
                value: json!("rel_123"),
            },
            RequiredField {
                key: "config_instance_ids",
                value: json!(["cfg_1", "cfg_2"]),
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
            OptionalField {
                key: "attempts",
                value: json!(3),
                default_value: json!(0),
            },
            OptionalField {
                key: "cooldown_ends_at",
                value: json!("2099-01-01T00:00:00Z"),
                default_value: json!(null),
            },
        ]
    }
}

serde_tests!(Deployment);

#[test]
fn status_method() {
    let deployment = Deployment {
        activity_status: DeploymentActivityStatus::Deployed,
        error_status: DeploymentErrorStatus::None,
        ..Default::default()
    };
    assert_eq!(deployment.status(), DeploymentStatus::Deployed);

    let deployment = Deployment {
        activity_status: DeploymentActivityStatus::Staged,
        error_status: DeploymentErrorStatus::Retrying,
        ..Default::default()
    };
    assert_eq!(deployment.status(), DeploymentStatus::Retrying);
}

#[test]
fn is_in_cooldown_method() {
    let deployment = Deployment::default();
    assert!(!deployment.is_in_cooldown());

    let mut deployment = Deployment::default();
    deployment.set_cooldown(TimeDelta::hours(1));
    assert!(deployment.is_in_cooldown());

    let deployment = Deployment {
        cooldown_ends_at: Some(Utc::now() - TimeDelta::hours(1)),
        ..Default::default()
    };
    assert!(!deployment.is_in_cooldown());
}

#[test]
fn set_cooldown_method() {
    let mut deployment = Deployment::default();
    assert!(deployment.cooldown_ends_at.is_none());

    deployment.set_cooldown(TimeDelta::seconds(60));
    assert!(deployment.cooldown_ends_at.is_some());

    let cooldown_end = deployment.cooldown_ends_at.unwrap();
    let expected_approx = Utc::now() + TimeDelta::seconds(60);
    let diff = (cooldown_end - expected_approx).num_seconds().abs();
    assert!(diff < 2, "cooldown_ends_at should be ~60s from now");
}

#[test]
fn attempts_method() {
    let deployment = Deployment::default();
    assert_eq!(deployment.attempts(), 0);

    let deployment = Deployment {
        attempts: 1,
        ..Default::default()
    };
    assert_eq!(deployment.attempts(), 1);
}

#[test]
fn defaults() {
    let actual = Deployment::default();

    let id = actual.id.clone();
    assert!(id.starts_with("unknown-"));
    let device_id = actual.device_id.clone();
    assert!(device_id.starts_with("unknown-"));
    let release_id = actual.release_id.clone();
    assert!(release_id.starts_with("unknown-"));
    let expected = Deployment {
        id,
        description: String::new(),
        status: DeploymentStatus::Staged,
        activity_status: DeploymentActivityStatus::Staged,
        error_status: DeploymentErrorStatus::None,
        target_status: DeploymentTargetStatus::Staged,
        device_id,
        release_id,
        created_at: DateTime::<Utc>::UNIX_EPOCH,
        updated_at: DateTime::<Utc>::UNIX_EPOCH,
        attempts: 0,
        cooldown_ends_at: None,
        config_instance_ids: Vec::new(),
    };

    assert_eq!(actual, expected);
}

// ─── target status enum tests ─────────────────────────────────────────────

impl StatusFixture for DeploymentTargetStatus {
    fn variants() -> Vec<Self> {
        DeploymentTargetStatus::variants()
    }
    fn cases() -> Vec<StatusCase<Self>> {
        vec![
            StatusCase {
                input: "\"staged\"",
                expected: DeploymentTargetStatus::Staged,
                valid: true,
            },
            StatusCase {
                input: "\"deployed\"",
                expected: DeploymentTargetStatus::Deployed,
                valid: true,
            },
            StatusCase {
                input: "\"archived\"",
                expected: DeploymentTargetStatus::Archived,
                valid: true,
            },
            StatusCase {
                input: "\"unknown\"",
                expected: DeploymentTargetStatus::Staged,
                valid: false,
            },
        ]
    }
}

mod target_status {
    use super::*;
    status_serde_tests!(DeploymentTargetStatus);
}

#[test]
fn target_status_backend_conversions() {
    struct TestCase {
        storage: DeploymentTargetStatus,
        backend: backend_client::DeploymentTargetStatus,
    }

    let test_cases = vec![
        TestCase {
            storage: DeploymentTargetStatus::Staged,
            backend: backend_client::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_STAGED,
        },
        TestCase {
            storage: DeploymentTargetStatus::Deployed,
            backend: backend_client::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_DEPLOYED,
        },
        TestCase {
            storage: DeploymentTargetStatus::Archived,
            backend: backend_client::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_ARCHIVED,
        },
    ];

    for test_case in test_cases {
        assert_eq!(
            DeploymentTargetStatus::from_backend(&test_case.backend),
            test_case.storage
        );
        assert_eq!(
            DeploymentTargetStatus::to_backend(&test_case.storage),
            test_case.backend
        );
    }
}

// ─── activity status enum tests ──────────────────────────────────────────────
impl StatusFixture for DeploymentActivityStatus {
    fn variants() -> Vec<Self> {
        DeploymentActivityStatus::variants()
    }
    fn cases() -> Vec<StatusCase<Self>> {
        vec![
            StatusCase {
                input: "\"drifted\"",
                expected: DeploymentActivityStatus::Drifted,
                valid: true,
            },
            StatusCase {
                input: "\"staged\"",
                expected: DeploymentActivityStatus::Staged,
                valid: true,
            },
            StatusCase {
                input: "\"queued\"",
                expected: DeploymentActivityStatus::Queued,
                valid: true,
            },
            StatusCase {
                input: "\"deployed\"",
                expected: DeploymentActivityStatus::Deployed,
                valid: true,
            },
            StatusCase {
                input: "\"archived\"",
                expected: DeploymentActivityStatus::Archived,
                valid: true,
            },
            StatusCase {
                input: "\"unknown\"",
                expected: DeploymentActivityStatus::Drifted,
                valid: false,
            },
        ]
    }
}

mod activity_status {
    use super::*;
    status_serde_tests!(DeploymentActivityStatus);
}

#[test]
fn activity_status_backend_conversions() {
    struct TestCase {
        storage: DeploymentActivityStatus,
        backend: backend_client::DeploymentActivityStatus,
    }

    let test_cases = vec![
        TestCase {
            storage: DeploymentActivityStatus::Drifted,
            backend: backend_client::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_DRIFTED,
        },
        TestCase {
            storage: DeploymentActivityStatus::Staged,
            backend: backend_client::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_STAGED,
        },
        TestCase {
            storage: DeploymentActivityStatus::Queued,
            backend: backend_client::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_QUEUED,
        },
        TestCase {
            storage: DeploymentActivityStatus::Deployed,
            backend: backend_client::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_DEPLOYED,
        },
        TestCase {
            storage: DeploymentActivityStatus::Archived,
            backend: backend_client::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_ARCHIVED,
        },
    ];

    for test_case in test_cases {
        assert_eq!(
            DeploymentActivityStatus::from_backend(&test_case.backend),
            test_case.storage
        );
        assert_eq!(
            DeploymentActivityStatus::to_backend(&test_case.storage),
            test_case.backend
        );
    }
}

// ─── error status enum tests ──────────────────────────────────────────────
impl StatusFixture for DeploymentErrorStatus {
    fn variants() -> Vec<Self> {
        DeploymentErrorStatus::variants()
    }
    fn cases() -> Vec<StatusCase<Self>> {
        vec![
            StatusCase {
                input: "\"none\"",
                expected: DeploymentErrorStatus::None,
                valid: true,
            },
            StatusCase {
                input: "\"failed\"",
                expected: DeploymentErrorStatus::Failed,
                valid: true,
            },
            StatusCase {
                input: "\"retrying\"",
                expected: DeploymentErrorStatus::Retrying,
                valid: true,
            },
            StatusCase {
                input: "\"unknown\"",
                expected: DeploymentErrorStatus::None,
                valid: false,
            },
        ]
    }
}

mod error_status {
    use super::*;
    status_serde_tests!(DeploymentErrorStatus);
}

#[test]
fn error_status_backend_conversions() {
    struct TestCase {
        storage: DeploymentErrorStatus,
        backend: backend_client::DeploymentErrorStatus,
    }

    let test_cases = vec![
        TestCase {
            storage: DeploymentErrorStatus::None,
            backend: backend_client::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_NONE,
        },
        TestCase {
            storage: DeploymentErrorStatus::Failed,
            backend: backend_client::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_FAILED,
        },
        TestCase {
            storage: DeploymentErrorStatus::Retrying,
            backend: backend_client::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_RETRYING,
        },
    ];

    for test_case in test_cases {
        assert_eq!(
            DeploymentErrorStatus::from_backend(&test_case.backend),
            test_case.storage
        );
        assert_eq!(
            DeploymentErrorStatus::to_backend(&test_case.storage),
            test_case.backend
        );
    }
}

impl StatusFixture for DeploymentStatus {
    fn variants() -> Vec<Self> {
        DeploymentStatus::variants()
    }
    fn cases() -> Vec<StatusCase<Self>> {
        vec![
            StatusCase {
                input: "\"drifted\"",
                expected: DeploymentStatus::Drifted,
                valid: true,
            },
            StatusCase {
                input: "\"staged\"",
                expected: DeploymentStatus::Staged,
                valid: true,
            },
            StatusCase {
                input: "\"queued\"",
                expected: DeploymentStatus::Queued,
                valid: true,
            },
            StatusCase {
                input: "\"deployed\"",
                expected: DeploymentStatus::Deployed,
                valid: true,
            },
            StatusCase {
                input: "\"archived\"",
                expected: DeploymentStatus::Archived,
                valid: true,
            },
            StatusCase {
                input: "\"failed\"",
                expected: DeploymentStatus::Failed,
                valid: true,
            },
            StatusCase {
                input: "\"retrying\"",
                expected: DeploymentStatus::Retrying,
                valid: true,
            },
            StatusCase {
                input: "\"unknown\"",
                expected: DeploymentStatus::Drifted,
                valid: false,
            },
        ]
    }
}

mod status {
    use super::*;
    status_serde_tests!(DeploymentStatus);
}

#[test]
fn status_backend_conversion() {
    struct TestCase {
        storage: DeploymentStatus,
        backend: backend_client::DeploymentStatus,
    }

    let test_cases = vec![
        TestCase {
            storage: DeploymentStatus::Drifted,
            backend: backend_client::DeploymentStatus::DEPLOYMENT_STATUS_DRIFTED,
        },
        TestCase {
            storage: DeploymentStatus::Staged,
            backend: backend_client::DeploymentStatus::DEPLOYMENT_STATUS_STAGED,
        },
        TestCase {
            storage: DeploymentStatus::Queued,
            backend: backend_client::DeploymentStatus::DEPLOYMENT_STATUS_QUEUED,
        },
        TestCase {
            storage: DeploymentStatus::Deployed,
            backend: backend_client::DeploymentStatus::DEPLOYMENT_STATUS_DEPLOYED,
        },
        TestCase {
            storage: DeploymentStatus::Archived,
            backend: backend_client::DeploymentStatus::DEPLOYMENT_STATUS_ARCHIVED,
        },
        TestCase {
            storage: DeploymentStatus::Failed,
            backend: backend_client::DeploymentStatus::DEPLOYMENT_STATUS_FAILED,
        },
        TestCase {
            storage: DeploymentStatus::Retrying,
            backend: backend_client::DeploymentStatus::DEPLOYMENT_STATUS_RETRYING,
        },
    ];

    for test_case in test_cases {
        assert_eq!(
            DeploymentStatus::from_backend(&test_case.backend),
            test_case.storage
        );
        assert_eq!(
            DeploymentStatus::to_backend(&test_case.storage),
            test_case.backend
        );
    }
}

#[test]
fn status_from_activity_and_error() {
    struct TestCase {
        activity_status: DeploymentActivityStatus,
        error_status: DeploymentErrorStatus,
        expected: DeploymentStatus,
    }

    let test_cases = vec![
        TestCase {
            activity_status: DeploymentActivityStatus::Drifted,
            error_status: DeploymentErrorStatus::None,
            expected: DeploymentStatus::Drifted,
        },
        TestCase {
            activity_status: DeploymentActivityStatus::Staged,
            error_status: DeploymentErrorStatus::None,
            expected: DeploymentStatus::Staged,
        },
        TestCase {
            activity_status: DeploymentActivityStatus::Queued,
            error_status: DeploymentErrorStatus::None,
            expected: DeploymentStatus::Queued,
        },
        TestCase {
            activity_status: DeploymentActivityStatus::Deployed,
            error_status: DeploymentErrorStatus::None,
            expected: DeploymentStatus::Deployed,
        },
        TestCase {
            activity_status: DeploymentActivityStatus::Archived,
            error_status: DeploymentErrorStatus::None,
            expected: DeploymentStatus::Archived,
        },
        TestCase {
            activity_status: DeploymentActivityStatus::Deployed,
            error_status: DeploymentErrorStatus::Retrying,
            expected: DeploymentStatus::Retrying,
        },
        TestCase {
            activity_status: DeploymentActivityStatus::Staged,
            error_status: DeploymentErrorStatus::Failed,
            expected: DeploymentStatus::Failed,
        },
    ];

    for test_case in test_cases {
        let result = DeploymentStatus::from_activity_and_error(
            &test_case.activity_status,
            &test_case.error_status,
        );
        assert_eq!(result, test_case.expected);
    }
}

// ─── from-backend tests ──────────────────────────────────────────────────────

#[test]
fn from_backend() {
    let now = Utc::now();
    let backend_deployment = backend_client::Deployment {
        object: backend_client::deployment::Object::Deployment,
        id: "dpl_123".to_string(),
        description: "Test deployment".to_string(),
        status: backend_client::DeploymentStatus::DEPLOYMENT_STATUS_STAGED,
        activity_status:
            backend_client::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_STAGED,
        error_status: backend_client::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_NONE,
        target_status: backend_client::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_STAGED,
        device_id: "device_123".to_string(),
        release_id: "rel_123".to_string(),
        created_at: now.to_rfc3339(),
        updated_at: now.to_rfc3339(),
        release: None,
        config_instances: Some(vec![
            backend_client::ConfigInstance {
                object: backend_client::config_instance::Object::ConfigInstance,
                id: "cfg_1".to_string(),
                ..Default::default()
            },
            backend_client::ConfigInstance {
                object: backend_client::config_instance::Object::ConfigInstance,
                id: "cfg_2".to_string(),
                ..Default::default()
            },
        ]),
    };

    let actual = Deployment::from_backend(backend_deployment);

    let expected = Deployment {
        id: "dpl_123".to_string(),
        description: "Test deployment".to_string(),
        status: DeploymentStatus::Staged,
        activity_status: DeploymentActivityStatus::Staged,
        error_status: DeploymentErrorStatus::None,
        target_status: DeploymentTargetStatus::Staged,
        device_id: "device_123".to_string(),
        release_id: "rel_123".to_string(),
        created_at: now,
        updated_at: now,
        attempts: 0,
        cooldown_ends_at: None,
        config_instance_ids: vec!["cfg_1".to_string(), "cfg_2".to_string()],
    };
    assert_eq!(actual, expected);
}

#[test]
fn from_backend_invalid_dates() {
    let backend_deployment = backend_client::Deployment {
        object: backend_client::deployment::Object::Deployment,
        id: "dpl_bad_dates".to_string(),
        description: "Bad dates".to_string(),
        status: backend_client::DeploymentStatus::DEPLOYMENT_STATUS_STAGED,
        activity_status:
            backend_client::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_STAGED,
        error_status: backend_client::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_NONE,
        target_status: backend_client::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_STAGED,
        device_id: "device_123".to_string(),
        release_id: "rel_123".to_string(),
        created_at: "not-a-date".to_string(),
        updated_at: "also-not-a-date".to_string(),
        release: None,
        config_instances: None,
    };

    let deployment = Deployment::from_backend(backend_deployment);
    assert_eq!(deployment.created_at, DateTime::<Utc>::UNIX_EPOCH);
    assert_eq!(deployment.updated_at, DateTime::<Utc>::UNIX_EPOCH);
}
