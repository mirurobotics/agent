// standard library
use std::collections::HashSet;

// internal crates
use miru_agent::models::device::{Device, DeviceStatus, Updates};
use miru_agent::models::Mergeable;

// external crates
use chrono::{DateTime, Duration, Utc};
use serde_json::json;

// harness
use crate::models::harnesses::{
    serde_tests, status_serde_tests, ModelFixture, OptionalField, RequiredField, StatusCase,
    StatusFixture,
};

// ─── fixture ─────────────────────────────────────────────────────────────────

impl ModelFixture for Device {
    fn required_fields() -> Vec<RequiredField> {
        vec![
            RequiredField {
                key: "device_id",
                value: json!("123"),
            },
            RequiredField {
                key: "session_id",
                value: json!("123"),
            },
        ]
    }

    fn optional_fields() -> Vec<OptionalField> {
        vec![
            OptionalField {
                key: "name",
                value: json!("test"),
                default_value: json!("placeholder"),
            },
            OptionalField {
                key: "agent_version",
                value: json!("v1.0.0"),
                default_value: json!("placeholder"),
            },
            OptionalField {
                key: "activated",
                value: json!(true),
                default_value: json!(false),
            },
            OptionalField {
                key: "status",
                value: json!("online"),
                default_value: json!("offline"),
            },
            OptionalField {
                key: "last_synced_at",
                value: json!("2023-11-14T22:13:20Z"),
                default_value: json!("1970-01-01T00:00:00Z"),
            },
            OptionalField {
                key: "last_connected_at",
                value: json!("2023-11-14T22:15:00Z"),
                default_value: json!("1970-01-01T00:00:00Z"),
            },
            OptionalField {
                key: "last_disconnected_at",
                value: json!("2023-11-14T22:16:40Z"),
                default_value: json!("1970-01-01T00:00:00Z"),
            },
        ]
    }
}

serde_tests!(Device);

#[test]
fn defaults() {
    let device = Device::default();

    let expected = Device {
        id: "placeholder".to_string(),
        session_id: "placeholder".to_string(),
        name: "placeholder".to_string(),
        agent_version: "placeholder".to_string(),
        activated: false,
        status: DeviceStatus::Offline,
        last_synced_at: DateTime::<Utc>::UNIX_EPOCH,
        last_connected_at: DateTime::<Utc>::UNIX_EPOCH,
        last_disconnected_at: DateTime::<Utc>::UNIX_EPOCH,
    };
    assert_eq!(device, expected);
}

// ─── status enum tests ───────────────────────────────────────────────────────

impl StatusFixture for DeviceStatus {
    fn variants() -> Vec<Self> {
        DeviceStatus::variants()
    }
    fn cases() -> Vec<StatusCase<Self>> {
        vec![
            StatusCase {
                input: "\"online\"",
                expected: DeviceStatus::Online,
                valid: true,
            },
            StatusCase {
                input: "\"offline\"",
                expected: DeviceStatus::Offline,
                valid: true,
            },
            StatusCase {
                input: "\"unknown\"",
                expected: DeviceStatus::Offline,
                valid: false,
            },
        ]
    }
}

mod status {
    use super::*;
    status_serde_tests!(DeviceStatus);
}

#[test]
fn status_default() {
    let status = DeviceStatus::default();
    assert_eq!(status, DeviceStatus::Offline);
}

#[test]
fn status_sdk_conversion() {
    struct TestCase {
        storage: DeviceStatus,
        sdk: openapi_server::models::DeviceStatus,
    }

    let test_cases = vec![
        TestCase {
            storage: DeviceStatus::Online,
            sdk: openapi_server::models::DeviceStatus::DEVICE_STATUS_ONLINE,
        },
        TestCase {
            storage: DeviceStatus::Offline,
            sdk: openapi_server::models::DeviceStatus::DEVICE_STATUS_OFFLINE,
        },
    ];

    let mut variants = DeviceStatus::variants().into_iter().collect::<HashSet<_>>();

    for test_case in test_cases {
        variants.remove(&test_case.storage);
        assert_eq!(test_case.sdk, DeviceStatus::to_sdk(&test_case.storage));
    }

    assert!(variants.is_empty(), "variants: {variants:?}");
}

// ─── merge tests ──────────────────────────────────────────────────────────────

#[test]
fn merge_empty() {
    let initial = Device {
        id: "123".to_string(),
        session_id: "123".to_string(),
        name: "test".to_string(),
        agent_version: "v1.0.0".to_string(),
        activated: true,
        status: DeviceStatus::Online,
        last_synced_at: Utc::now(),
        last_connected_at: Utc::now(),
        last_disconnected_at: Utc::now(),
    };
    let updates = Updates::empty();
    let expected = initial.clone();
    let mut actual = initial.clone();
    actual.merge(updates);
    assert_eq!(expected, actual);
}

#[test]
fn merge_all() {
    let initial = Device {
        id: "123".to_string(),
        session_id: "123".to_string(),
        name: "test".to_string(),
        agent_version: "v1.0.0".to_string(),
        activated: true,
        status: DeviceStatus::Online,
        last_synced_at: Utc::now(),
        last_connected_at: Utc::now(),
        last_disconnected_at: Utc::now(),
    };
    let updates = Updates {
        id: Some("456".to_string()),
        name: Some("test2".to_string()),
        activated: Some(false),
        agent_version: Some("v1.0.1".to_string()),
        status: Some(DeviceStatus::Offline),
        last_synced_at: Some(Utc::now() + Duration::days(1)),
        last_connected_at: Some(Utc::now() + Duration::days(1)),
        last_disconnected_at: Some(Utc::now() + Duration::days(1)),
    };
    let expected = Device {
        id: updates.id.clone().unwrap(),
        session_id: initial.session_id.clone(),
        name: updates.name.clone().unwrap(),
        agent_version: updates.agent_version.clone().unwrap(),
        activated: updates.activated.unwrap(),
        status: updates.status.clone().unwrap(),
        last_synced_at: updates.last_synced_at.unwrap(),
        last_connected_at: updates.last_connected_at.unwrap(),
        last_disconnected_at: updates.last_disconnected_at.unwrap(),
    };
    let mut actual = initial.clone();
    actual.merge(updates);
    assert_eq!(expected, actual);
}

// ─── updates tests ────────────────────────────────────────────────────────────
#[test]
fn updates_empty() {
    let actual = Updates::empty();
    let expected = Updates {
        id: None,
        name: None,
        agent_version: None,
        activated: None,
        status: None,
        last_synced_at: None,
        last_connected_at: None,
        last_disconnected_at: None,
    };
    assert_eq!(actual, expected);
}

#[test]
fn updates_disconnected() {
    let before = Utc::now();
    let actual = Updates::disconnected();
    let after = Utc::now();

    let disconnected_at = actual.last_disconnected_at.unwrap();
    assert!(disconnected_at >= before && disconnected_at <= after);

    let expected = Updates {
        status: Some(DeviceStatus::Offline),
        last_disconnected_at: Some(disconnected_at),
        ..Updates::empty()
    };

    assert_eq!(actual, expected);
}

#[test]
fn updates_connected() {
    let before = Utc::now();
    let actual = Updates::connected();
    let after = Utc::now();

    let connected_at = actual.last_connected_at.unwrap();
    assert!(connected_at >= before && connected_at <= after);

    let expected = Updates {
        status: Some(DeviceStatus::Online),
        last_connected_at: Some(connected_at),
        ..Updates::empty()
    };
    assert_eq!(actual, expected);
}

#[test]
fn updates_set_agent_version() {
    let updates = Updates::set_agent_version("v2.0.0".to_string());
    let expected = Updates {
        agent_version: Some("v2.0.0".to_string()),
        ..Updates::empty()
    };
    assert_eq!(updates, expected);
}
