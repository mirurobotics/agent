// standard library
use std::collections::HashSet;

// internal crates
use miru_agent::models::deployment::{
    Deployment, DeploymentActivityStatus, DeploymentErrorStatus, DeploymentStatus,
    DeploymentTargetStatus,
};
use openapi_client::models::{
    Deployment as BackendDeployment, DeploymentActivityStatus as BackendDeploymentActivityStatus,
    DeploymentErrorStatus as BackendDeploymentErrorStatus, DeploymentStatus as BackendDeploymentStatus,
    DeploymentTargetStatus as BackendDeploymentTargetStatus,
};

// external crates
use chrono::{DateTime, Utc};
use serde_json::json;

#[test]
fn serialize_deserialize_deployment_target_status() {
    struct TestCase {
        input: &'static str,
        expected: DeploymentTargetStatus,
        valid: bool,
    }

    let test_cases = vec![
        TestCase {
            input: "\"staged\"",
            expected: DeploymentTargetStatus::Staged,
            valid: true,
        },
        TestCase {
            input: "\"deployed\"",
            expected: DeploymentTargetStatus::Deployed,
            valid: true,
        },
        TestCase {
            input: "\"archived\"",
            expected: DeploymentTargetStatus::Archived,
            valid: true,
        },
        TestCase {
            input: "\"unknown\"",
            expected: DeploymentTargetStatus::Staged,
            valid: false,
        },
    ];

    let mut variants = DeploymentTargetStatus::variants().into_iter().collect::<HashSet<_>>();

    for test_case in test_cases {
        variants.remove(&test_case.expected);
        let deserialized = serde_json::from_str::<DeploymentTargetStatus>(test_case.input).unwrap();
        assert_eq!(deserialized, test_case.expected);
        if test_case.valid {
            let serialized = serde_json::to_string(&test_case.expected).unwrap();
            assert_eq!(serialized, test_case.input);
        }
    }

    assert!(variants.is_empty(), "variants: {variants:?}");
}

#[test]
fn deployment_target_status_backend_conversions() {
    struct TestCase {
        storage: DeploymentTargetStatus,
        backend: BackendDeploymentTargetStatus,
    }

    let test_cases = vec![
        TestCase {
            storage: DeploymentTargetStatus::Staged,
            backend: BackendDeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_STAGED,
        },
        TestCase {
            storage: DeploymentTargetStatus::Deployed,
            backend: BackendDeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_DEPLOYED,
        },
        TestCase {
            storage: DeploymentTargetStatus::Archived,
            backend: BackendDeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_ARCHIVED,
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

#[test]
fn serialize_deserialize_deployment_activity_status() {
    struct TestCase {
        input: &'static str,
        expected: DeploymentActivityStatus,
        valid: bool,
    }

    let test_cases = vec![
        TestCase {
            input: "\"drifted\"",
            expected: DeploymentActivityStatus::Drifted,
            valid: true,
        },
        TestCase {
            input: "\"staged\"",
            expected: DeploymentActivityStatus::Staged,
            valid: true,
        },
        TestCase {
            input: "\"queued\"",
            expected: DeploymentActivityStatus::Queued,
            valid: true,
        },
        TestCase {
            input: "\"deployed\"",
            expected: DeploymentActivityStatus::Deployed,
            valid: true,
        },
        TestCase {
            input: "\"archived\"",
            expected: DeploymentActivityStatus::Archived,
            valid: true,
        },
        TestCase {
            input: "\"unknown\"",
            expected: DeploymentActivityStatus::Drifted,
            valid: false,
        },
    ];

    let mut variants = DeploymentActivityStatus::variants().into_iter().collect::<HashSet<_>>();

    for test_case in test_cases {
        variants.remove(&test_case.expected);
        let deserialized = serde_json::from_str::<DeploymentActivityStatus>(test_case.input).unwrap();
        assert_eq!(deserialized, test_case.expected);
        if test_case.valid {
            let serialized = serde_json::to_string(&test_case.expected).unwrap();
            assert_eq!(serialized, test_case.input);
        }
    }

    assert!(variants.is_empty(), "variants: {variants:?}");
}

#[test]
fn deployment_activity_status_backend_conversions() {
    struct TestCase {
        storage: DeploymentActivityStatus,
        backend: BackendDeploymentActivityStatus,
    }

    let test_cases = vec![
        TestCase {
            storage: DeploymentActivityStatus::Drifted,
            backend: BackendDeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_DRIFTED,
        },
        TestCase {
            storage: DeploymentActivityStatus::Staged,
            backend: BackendDeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_STAGED,
        },
        TestCase {
            storage: DeploymentActivityStatus::Queued,
            backend: BackendDeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_QUEUED,
        },
        TestCase {
            storage: DeploymentActivityStatus::Deployed,
            backend: BackendDeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_DEPLOYED,
        },
        TestCase {
            storage: DeploymentActivityStatus::Archived,
            backend: BackendDeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_ARCHIVED,
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

#[test]
fn serialize_deserialize_deployment_error_status() {
    struct TestCase {
        input: &'static str,
        expected: DeploymentErrorStatus,
        valid: bool,
    }

    let test_cases = vec![
        TestCase {
            input: "\"none\"",
            expected: DeploymentErrorStatus::None,
            valid: true,
        },
        TestCase {
            input: "\"failed\"",
            expected: DeploymentErrorStatus::Failed,
            valid: true,
        },
        TestCase {
            input: "\"retrying\"",
            expected: DeploymentErrorStatus::Retrying,
            valid: true,
        },
        TestCase {
            input: "\"unknown\"",
            expected: DeploymentErrorStatus::None,
            valid: false,
        },
    ];

    let mut variants = DeploymentErrorStatus::variants().into_iter().collect::<HashSet<_>>();

    for test_case in test_cases {
        variants.remove(&test_case.expected);
        let deserialized = serde_json::from_str::<DeploymentErrorStatus>(test_case.input).unwrap();
        assert_eq!(deserialized, test_case.expected);
        if test_case.valid {
            let serialized = serde_json::to_string(&test_case.expected).unwrap();
            assert_eq!(serialized, test_case.input);
        }
    }

    assert!(variants.is_empty(), "variants: {variants:?}");
}

#[test]
fn deployment_error_status_backend_conversions() {
    struct TestCase {
        storage: DeploymentErrorStatus,
        backend: BackendDeploymentErrorStatus,
    }

    let test_cases = vec![
        TestCase {
            storage: DeploymentErrorStatus::None,
            backend: BackendDeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_NONE,
        },
        TestCase {
            storage: DeploymentErrorStatus::Failed,
            backend: BackendDeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_FAILED,
        },
        TestCase {
            storage: DeploymentErrorStatus::Retrying,
            backend: BackendDeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_RETRYING,
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

#[test]
fn serialize_deserialize_deployment_status() {
    struct TestCase {
        input: &'static str,
        expected: DeploymentStatus,
        valid: bool,
    }

    let test_cases = vec![
        TestCase {
            input: "\"drifted\"",
            expected: DeploymentStatus::Drifted,
            valid: true,
        },
        TestCase {
            input: "\"staged\"",
            expected: DeploymentStatus::Staged,
            valid: true,
        },
        TestCase {
            input: "\"queued\"",
            expected: DeploymentStatus::Queued,
            valid: true,
        },
        TestCase {
            input: "\"deployed\"",
            expected: DeploymentStatus::Deployed,
            valid: true,
        },
        TestCase {
            input: "\"archived\"",
            expected: DeploymentStatus::Archived,
            valid: true,
        },
        TestCase {
            input: "\"failed\"",
            expected: DeploymentStatus::Failed,
            valid: true,
        },
        TestCase {
            input: "\"retrying\"",
            expected: DeploymentStatus::Retrying,
            valid: true,
        },
        TestCase {
            input: "\"unknown\"",
            expected: DeploymentStatus::Drifted,
            valid: false,
        },
    ];

    let mut variants = DeploymentStatus::variants().into_iter().collect::<HashSet<_>>();

    for test_case in test_cases {
        variants.remove(&test_case.expected);
        let deserialized = serde_json::from_str::<DeploymentStatus>(test_case.input).unwrap();
        assert_eq!(deserialized, test_case.expected);
        if test_case.valid {
            let serialized = serde_json::to_string(&test_case.expected).unwrap();
            assert_eq!(serialized, test_case.input);
        }
    }

    assert!(variants.is_empty(), "variants: {variants:?}");
}

#[test]
fn deployment_status_backend_conversions() {
    struct TestCase {
        storage: DeploymentStatus,
        backend: BackendDeploymentStatus,
    }

    let test_cases = vec![
        TestCase {
            storage: DeploymentStatus::Drifted,
            backend: BackendDeploymentStatus::DEPLOYMENT_STATUS_DRIFTED,
        },
        TestCase {
            storage: DeploymentStatus::Staged,
            backend: BackendDeploymentStatus::DEPLOYMENT_STATUS_STAGED,
        },
        TestCase {
            storage: DeploymentStatus::Queued,
            backend: BackendDeploymentStatus::DEPLOYMENT_STATUS_QUEUED,
        },
        TestCase {
            storage: DeploymentStatus::Deployed,
            backend: BackendDeploymentStatus::DEPLOYMENT_STATUS_DEPLOYED,
        },
        TestCase {
            storage: DeploymentStatus::Archived,
            backend: BackendDeploymentStatus::DEPLOYMENT_STATUS_ARCHIVED,
        },
        TestCase {
            storage: DeploymentStatus::Failed,
            backend: BackendDeploymentStatus::DEPLOYMENT_STATUS_FAILED,
        },
        TestCase {
            storage: DeploymentStatus::Retrying,
            backend: BackendDeploymentStatus::DEPLOYMENT_STATUS_RETRYING,
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
fn deployment_status_from_activity_and_error() {
    struct TestCase {
        activity_status: DeploymentActivityStatus,
        error_status: DeploymentErrorStatus,
        expected: DeploymentStatus,
    }

    let test_cases = vec![
        // No error cases
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
        // Error cases take precedence
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

#[test]
fn deployment_from_backend() {
    let now = Utc::now();
    let backend_deployment = BackendDeployment {
        object: openapi_client::models::deployment::Object::Deployment,
        id: "dpl_123".to_string(),
        description: "Test deployment".to_string(),
        status: BackendDeploymentStatus::DEPLOYMENT_STATUS_STAGED,
        activity_status: BackendDeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_STAGED,
        error_status: BackendDeploymentErrorStatus::DEPLOYMENT_ERROR_STATUS_NONE,
        target_status: BackendDeploymentTargetStatus::DEPLOYMENT_TARGET_STATUS_STAGED,
        device_id: "device_123".to_string(),
        release_id: "rel_123".to_string(),
        created_at: now.to_rfc3339(),
        updated_at: now.to_rfc3339(),
        release: None,
        config_instances: None,
    };

    let deployment = Deployment::from_backend(backend_deployment);

    assert_eq!(deployment.id, "dpl_123");
    assert_eq!(deployment.description, "Test deployment");
    assert_eq!(deployment.status, DeploymentStatus::Staged);
    assert_eq!(deployment.activity_status, DeploymentActivityStatus::Staged);
    assert_eq!(deployment.error_status, DeploymentErrorStatus::None);
    assert_eq!(deployment.target_status, DeploymentTargetStatus::Staged);
    assert_eq!(deployment.device_id, "device_123");
    assert_eq!(deployment.release_id, "rel_123");
    assert_eq!(deployment.release, None);
    assert_eq!(deployment.config_instances, None);
}

#[test]
fn deployment_status_method() {
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

