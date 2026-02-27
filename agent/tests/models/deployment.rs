// internal crates
use miru_agent::models::deployment::Updates;
use miru_agent::models::Patch;
use miru_agent::models::{Deployment, DplActivity, DplErrStatus, DplStatus, DplTarget};
use openapi_client::models as backend_client;
use openapi_server::models as agent_server;

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
                default_value: json!("1970-01-01T00:00:00Z"),
            },
        ]
    }
}

serde_tests!(Deployment);

#[test]
fn status_method() {
    let deployment = Deployment {
        activity_status: DplActivity::Deployed,
        error_status: DplErrStatus::None,
        ..Default::default()
    };
    assert_eq!(deployment.status(), DplStatus::Deployed);

    let deployment = Deployment {
        activity_status: DplActivity::Staged,
        error_status: DplErrStatus::Retrying,
        ..Default::default()
    };
    assert_eq!(deployment.status(), DplStatus::Retrying);
}

#[test]
fn is_in_cooldown_method() {
    let deployment = Deployment::default();
    assert!(!deployment.is_in_cooldown());

    let mut deployment = Deployment::default();
    deployment.set_cooldown(TimeDelta::hours(1));
    assert!(deployment.is_in_cooldown());

    let deployment = Deployment {
        cooldown_ends_at: Utc::now() - TimeDelta::hours(1),
        ..Default::default()
    };
    assert!(!deployment.is_in_cooldown());
}

#[test]
fn set_cooldown_method() {
    let mut deployment = Deployment::default();
    assert_eq!(deployment.cooldown_ends_at, DateTime::<Utc>::UNIX_EPOCH);

    deployment.set_cooldown(TimeDelta::seconds(60));

    let expected_approx = Utc::now() + TimeDelta::seconds(60);
    let diff = (deployment.cooldown_ends_at - expected_approx)
        .num_seconds()
        .abs();
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
        activity_status: DplActivity::Staged,
        error_status: DplErrStatus::None,
        target_status: DplTarget::Staged,
        device_id,
        release_id,
        created_at: DateTime::<Utc>::UNIX_EPOCH,
        updated_at: DateTime::<Utc>::UNIX_EPOCH,
        attempts: 0,
        cooldown_ends_at: DateTime::<Utc>::UNIX_EPOCH,
        deployed_at: None,
        archived_at: None,
        config_instance_ids: Vec::new(),
    };

    assert_eq!(actual, expected);
}

// ─── target status enum tests ─────────────────────────────────────────────

impl StatusFixture for DplTarget {
    fn variants() -> Vec<Self> {
        DplTarget::variants()
    }
    fn cases() -> Vec<StatusCase<Self>> {
        vec![
            StatusCase {
                input: "\"staged\"",
                expected: DplTarget::Staged,
                valid: true,
            },
            StatusCase {
                input: "\"deployed\"",
                expected: DplTarget::Deployed,
                valid: true,
            },
            StatusCase {
                input: "\"archived\"",
                expected: DplTarget::Archived,
                valid: true,
            },
            StatusCase {
                input: "\"unknown\"",
                expected: DplTarget::Staged,
                valid: false,
            },
        ]
    }
}

mod target_status {
    use super::*;
    status_serde_tests!(DplTarget);
}

#[test]
fn target_status_backend_conversions() {
    struct TestCase {
        storage: DplTarget,
        backend: backend_client::DeploymentTargetStatus,
    }

    let test_cases = vec![
        TestCase {
            storage: DplTarget::Staged,
            backend: backend_client::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_STAGED,
        },
        TestCase {
            storage: DplTarget::Deployed,
            backend: backend_client::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_DEPLOYED,
        },
        TestCase {
            storage: DplTarget::Archived,
            backend: backend_client::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_ARCHIVED,
        },
    ];

    for test_case in test_cases {
        assert_eq!(
            Into::<DplTarget>::into(&test_case.backend),
            test_case.storage
        );
        assert_eq!(
            Into::<backend_client::DeploymentTargetStatus>::into(&test_case.storage),
            test_case.backend
        );
    }
}

#[test]
fn target_status_sdk_conversions() {
    struct TestCase {
        storage: DplTarget,
        sdk: agent_server::DeploymentTargetStatus,
    }

    let test_cases = vec![
        TestCase {
            storage: DplTarget::Staged,
            sdk: agent_server::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_STAGED,
        },
        TestCase {
            storage: DplTarget::Deployed,
            sdk: agent_server::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_DEPLOYED,
        },
        TestCase {
            storage: DplTarget::Archived,
            sdk: agent_server::DeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_ARCHIVED,
        },
    ];

    for test_case in test_cases {
        assert_eq!(
            Into::<agent_server::DeploymentTargetStatus>::into(&test_case.storage),
            test_case.sdk
        );
    }
}

// ─── activity status enum tests ──────────────────────────────────────────────
impl StatusFixture for DplActivity {
    fn variants() -> Vec<Self> {
        DplActivity::variants()
    }
    fn cases() -> Vec<StatusCase<Self>> {
        vec![
            StatusCase {
                input: "\"drifted\"",
                expected: DplActivity::Drifted,
                valid: true,
            },
            StatusCase {
                input: "\"staged\"",
                expected: DplActivity::Staged,
                valid: true,
            },
            StatusCase {
                input: "\"queued\"",
                expected: DplActivity::Queued,
                valid: true,
            },
            StatusCase {
                input: "\"deployed\"",
                expected: DplActivity::Deployed,
                valid: true,
            },
            StatusCase {
                input: "\"archived\"",
                expected: DplActivity::Archived,
                valid: true,
            },
            StatusCase {
                input: "\"unknown\"",
                expected: DplActivity::Drifted,
                valid: false,
            },
        ]
    }
}

mod activity_status {
    use super::*;
    status_serde_tests!(DplActivity);
}

#[test]
fn activity_status_backend_conversions() {
    struct TestCase {
        storage: DplActivity,
        backend: backend_client::DeploymentActivityStatus,
    }

    let test_cases = vec![
        TestCase {
            storage: DplActivity::Drifted,
            backend: backend_client::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_DRIFTED,
        },
        TestCase {
            storage: DplActivity::Staged,
            backend: backend_client::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_STAGED,
        },
        TestCase {
            storage: DplActivity::Queued,
            backend: backend_client::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_QUEUED,
        },
        TestCase {
            storage: DplActivity::Deployed,
            backend: backend_client::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_DEPLOYED,
        },
        TestCase {
            storage: DplActivity::Archived,
            backend: backend_client::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_ARCHIVED,
        },
    ];

    for test_case in test_cases {
        assert_eq!(
            Into::<DplActivity>::into(&test_case.backend),
            test_case.storage
        );
        assert_eq!(
            Into::<backend_client::DeploymentActivityStatus>::into(&test_case.storage),
            test_case.backend
        );
    }
}

#[test]
fn activity_status_sdk_conversions() {
    struct TestCase {
        storage: DplActivity,
        sdk: agent_server::DeploymentActivityStatus,
    }

    let test_cases = vec![
        TestCase {
            storage: DplActivity::Drifted,
            sdk: agent_server::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_DRIFTED,
        },
        TestCase {
            storage: DplActivity::Staged,
            sdk: agent_server::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_STAGED,
        },
        TestCase {
            storage: DplActivity::Queued,
            sdk: agent_server::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_QUEUED,
        },
        TestCase {
            storage: DplActivity::Deployed,
            sdk: agent_server::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_DEPLOYED,
        },
        TestCase {
            storage: DplActivity::Archived,
            sdk: agent_server::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_ARCHIVED,
        },
    ];

    for test_case in test_cases {
        assert_eq!(
            Into::<agent_server::DeploymentActivityStatus>::into(&test_case.storage),
            test_case.sdk
        );
    }
}

// ─── error status enum tests ──────────────────────────────────────────────
impl StatusFixture for DplErrStatus {
    fn variants() -> Vec<Self> {
        DplErrStatus::variants()
    }
    fn cases() -> Vec<StatusCase<Self>> {
        vec![
            StatusCase {
                input: "\"none\"",
                expected: DplErrStatus::None,
                valid: true,
            },
            StatusCase {
                input: "\"failed\"",
                expected: DplErrStatus::Failed,
                valid: true,
            },
            StatusCase {
                input: "\"retrying\"",
                expected: DplErrStatus::Retrying,
                valid: true,
            },
            StatusCase {
                input: "\"unknown\"",
                expected: DplErrStatus::None,
                valid: false,
            },
        ]
    }
}

mod error_status {
    use super::*;
    status_serde_tests!(DplErrStatus);
}

#[test]
fn error_status_backend_conversions() {
    struct TestCase {
        storage: DplErrStatus,
        backend: backend_client::DeploymentErrorStatus,
    }

    let test_cases = vec![
        TestCase {
            storage: DplErrStatus::None,
            backend: backend_client::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_NONE,
        },
        TestCase {
            storage: DplErrStatus::Failed,
            backend: backend_client::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_FAILED,
        },
        TestCase {
            storage: DplErrStatus::Retrying,
            backend: backend_client::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_RETRYING,
        },
    ];

    for test_case in test_cases {
        assert_eq!(
            Into::<DplErrStatus>::into(&test_case.backend),
            test_case.storage
        );
        assert_eq!(
            Into::<backend_client::DeploymentErrorStatus>::into(&test_case.storage),
            test_case.backend
        );
    }
}

#[test]
fn error_status_sdk_conversions() {
    struct TestCase {
        storage: DplErrStatus,
        sdk: agent_server::DeploymentErrorStatus,
    }

    let test_cases = vec![
        TestCase {
            storage: DplErrStatus::None,
            sdk: agent_server::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_NONE,
        },
        TestCase {
            storage: DplErrStatus::Failed,
            sdk: agent_server::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_FAILED,
        },
        TestCase {
            storage: DplErrStatus::Retrying,
            sdk: agent_server::DeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_RETRYING,
        },
    ];

    for test_case in test_cases {
        assert_eq!(
            Into::<agent_server::DeploymentErrorStatus>::into(&test_case.storage),
            test_case.sdk
        );
    }
}

impl StatusFixture for DplStatus {
    fn variants() -> Vec<Self> {
        DplStatus::variants()
    }
    fn cases() -> Vec<StatusCase<Self>> {
        vec![
            StatusCase {
                input: "\"drifted\"",
                expected: DplStatus::Drifted,
                valid: true,
            },
            StatusCase {
                input: "\"staged\"",
                expected: DplStatus::Staged,
                valid: true,
            },
            StatusCase {
                input: "\"queued\"",
                expected: DplStatus::Queued,
                valid: true,
            },
            StatusCase {
                input: "\"deployed\"",
                expected: DplStatus::Deployed,
                valid: true,
            },
            StatusCase {
                input: "\"archived\"",
                expected: DplStatus::Archived,
                valid: true,
            },
            StatusCase {
                input: "\"failed\"",
                expected: DplStatus::Failed,
                valid: true,
            },
            StatusCase {
                input: "\"retrying\"",
                expected: DplStatus::Retrying,
                valid: true,
            },
            StatusCase {
                input: "\"unknown\"",
                expected: DplStatus::Drifted,
                valid: false,
            },
        ]
    }
}

mod status {
    use super::*;
    status_serde_tests!(DplStatus);
}

#[test]
fn status_backend_conversion() {
    struct TestCase {
        storage: DplStatus,
        backend: backend_client::DeploymentStatus,
    }

    let test_cases = vec![
        TestCase {
            storage: DplStatus::Drifted,
            backend: backend_client::DeploymentStatus::DEPLOYMENT_STATUS_DRIFTED,
        },
        TestCase {
            storage: DplStatus::Staged,
            backend: backend_client::DeploymentStatus::DEPLOYMENT_STATUS_STAGED,
        },
        TestCase {
            storage: DplStatus::Queued,
            backend: backend_client::DeploymentStatus::DEPLOYMENT_STATUS_QUEUED,
        },
        TestCase {
            storage: DplStatus::Deployed,
            backend: backend_client::DeploymentStatus::DEPLOYMENT_STATUS_DEPLOYED,
        },
        TestCase {
            storage: DplStatus::Archived,
            backend: backend_client::DeploymentStatus::DEPLOYMENT_STATUS_ARCHIVED,
        },
        TestCase {
            storage: DplStatus::Failed,
            backend: backend_client::DeploymentStatus::DEPLOYMENT_STATUS_FAILED,
        },
        TestCase {
            storage: DplStatus::Retrying,
            backend: backend_client::DeploymentStatus::DEPLOYMENT_STATUS_RETRYING,
        },
    ];

    for test_case in test_cases {
        assert_eq!(
            Into::<DplStatus>::into(&test_case.backend),
            test_case.storage
        );
        assert_eq!(
            Into::<backend_client::DeploymentStatus>::into(&test_case.storage),
            test_case.backend
        );
    }
}

#[test]
fn status_sdk_conversion() {
    struct TestCase {
        storage: DplStatus,
        sdk: agent_server::DeploymentStatus,
    }

    let test_cases = vec![
        TestCase {
            storage: DplStatus::Drifted,
            sdk: agent_server::DeploymentStatus::DEPLOYMENT_STATUS_DRIFTED,
        },
        TestCase {
            storage: DplStatus::Staged,
            sdk: agent_server::DeploymentStatus::DEPLOYMENT_STATUS_STAGED,
        },
        TestCase {
            storage: DplStatus::Queued,
            sdk: agent_server::DeploymentStatus::DEPLOYMENT_STATUS_QUEUED,
        },
        TestCase {
            storage: DplStatus::Deployed,
            sdk: agent_server::DeploymentStatus::DEPLOYMENT_STATUS_DEPLOYED,
        },
        TestCase {
            storage: DplStatus::Archived,
            sdk: agent_server::DeploymentStatus::DEPLOYMENT_STATUS_ARCHIVED,
        },
        TestCase {
            storage: DplStatus::Failed,
            sdk: agent_server::DeploymentStatus::DEPLOYMENT_STATUS_FAILED,
        },
        TestCase {
            storage: DplStatus::Retrying,
            sdk: agent_server::DeploymentStatus::DEPLOYMENT_STATUS_RETRYING,
        },
    ];

    for test_case in test_cases {
        assert_eq!(
            Into::<agent_server::DeploymentStatus>::into(&test_case.storage),
            test_case.sdk
        );
    }
}

#[test]
fn status_from_activity_and_error() {
    struct TestCase {
        activity_status: DplActivity,
        error_status: DplErrStatus,
        expected: DplStatus,
    }

    let test_cases = vec![
        TestCase {
            activity_status: DplActivity::Drifted,
            error_status: DplErrStatus::None,
            expected: DplStatus::Drifted,
        },
        TestCase {
            activity_status: DplActivity::Staged,
            error_status: DplErrStatus::None,
            expected: DplStatus::Staged,
        },
        TestCase {
            activity_status: DplActivity::Queued,
            error_status: DplErrStatus::None,
            expected: DplStatus::Queued,
        },
        TestCase {
            activity_status: DplActivity::Deployed,
            error_status: DplErrStatus::None,
            expected: DplStatus::Deployed,
        },
        TestCase {
            activity_status: DplActivity::Archived,
            error_status: DplErrStatus::None,
            expected: DplStatus::Archived,
        },
        TestCase {
            activity_status: DplActivity::Deployed,
            error_status: DplErrStatus::Retrying,
            expected: DplStatus::Retrying,
        },
        TestCase {
            activity_status: DplActivity::Staged,
            error_status: DplErrStatus::Failed,
            expected: DplStatus::Failed,
        },
    ];

    for test_case in test_cases {
        let result =
            DplStatus::from_activity_and_error(&test_case.activity_status, &test_case.error_status);
        assert_eq!(result, test_case.expected);
    }
}

// ─── update tests ────────────────────────────────────────────────────────────

#[test]
fn update_empty() {
    let initial = Deployment::default();
    let expected = initial.clone();
    let mut actual = initial;
    actual.patch(Updates::empty());
    assert_eq!(expected, actual);
}

#[test]
fn update_all() {
    let initial = Deployment::default();
    let now = Utc::now();
    let updates = Updates {
        activity_status: Some(DplActivity::Deployed),
        error_status: Some(DplErrStatus::Retrying),
        attempts: Some(5),
        cooldown: Some(TimeDelta::seconds(120)),
        deployed_at: Some(now),
        archived_at: Some(now),
    };
    let mut actual = initial.clone();
    actual.patch(updates);

    let diff = (actual.cooldown_ends_at - Utc::now()).num_seconds();
    assert!((diff - 120).abs() < 2, "cooldown should be ~120s from now");

    let expected = Deployment {
        activity_status: DplActivity::Deployed,
        error_status: DplErrStatus::Retrying,
        attempts: 5,
        cooldown_ends_at: actual.cooldown_ends_at,
        deployed_at: Some(now),
        archived_at: Some(now),
        ..initial
    };
    assert_eq!(actual, expected);
}

#[test]
fn update_partial() {
    let initial = Deployment {
        activity_status: DplActivity::Queued,
        error_status: DplErrStatus::None,
        attempts: 3,
        ..Default::default()
    };
    let updates = Updates {
        activity_status: Some(DplActivity::Deployed),
        ..Updates::empty()
    };
    let mut actual = initial.clone();
    actual.patch(updates);

    let expected = Deployment {
        activity_status: DplActivity::Deployed,
        ..initial
    };
    assert_eq!(actual, expected);
}

#[test]
fn updates_empty() {
    let actual = Updates::empty();
    let expected = Updates {
        activity_status: None,
        error_status: None,
        attempts: None,
        cooldown: None,
        deployed_at: None,
        archived_at: None,
    };
    assert_eq!(actual, expected);
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

    let config_instance_ids: Vec<String> = backend_deployment
        .config_instances
        .as_ref()
        .unwrap()
        .iter()
        .map(|ci| ci.id.clone())
        .collect();
    let actual = Deployment::from_backend(backend_deployment, config_instance_ids);

    let expected = Deployment {
        id: "dpl_123".to_string(),
        description: "Test deployment".to_string(),
        activity_status: DplActivity::Staged,
        error_status: DplErrStatus::None,
        target_status: DplTarget::Staged,
        device_id: "device_123".to_string(),
        release_id: "rel_123".to_string(),
        created_at: now,
        updated_at: now,
        attempts: 0,
        cooldown_ends_at: DateTime::<Utc>::UNIX_EPOCH,
        deployed_at: None,
        archived_at: None,
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
        updated_at: "not-a-date".to_string(),
        release: None,
        config_instances: Some(vec![]),
    };

    let deployment = Deployment::from_backend(backend_deployment, vec![]);
    assert_eq!(deployment.created_at, DateTime::<Utc>::UNIX_EPOCH);
    assert_eq!(deployment.updated_at, DateTime::<Utc>::UNIX_EPOCH);
}
