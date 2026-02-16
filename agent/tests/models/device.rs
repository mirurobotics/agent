// standard library
use std::collections::HashSet;

// internal crates
use miru_agent::models::device::{Device, DeviceStatus, Updates};
use miru_agent::models::Mergeable;

// external crates
use chrono::{Duration, Utc};
use serde_json::json;
#[allow(unused_imports)]
use tracing::{debug, error, info, trace, warn};

#[test]
fn serialize_deserialize_device_status() {
    struct TestCase {
        input: &'static str,
        expected: DeviceStatus,
        valid: bool,
    }

    let test_cases = vec![
        TestCase {
            input: "\"online\"",
            expected: DeviceStatus::Online,
            valid: true,
        },
        TestCase {
            input: "\"offline\"",
            expected: DeviceStatus::Offline,
            valid: true,
        },
        // default
        TestCase {
            input: "\"unknown\"",
            expected: DeviceStatus::Offline,
            valid: false,
        },
    ];

    let mut variants = DeviceStatus::variants().into_iter().collect::<HashSet<_>>();

    for test_case in test_cases {
        variants.remove(&test_case.expected);
        let deserialized = serde_json::from_str::<DeviceStatus>(test_case.input).unwrap();
        assert_eq!(deserialized, test_case.expected);
        if test_case.valid {
            let serialized = serde_json::to_string(&test_case.expected).unwrap();
            assert_eq!(serialized, test_case.input);
        }
    }

    assert!(variants.is_empty(), "variants: {variants:?}");
}

#[test]
fn device_status_sdk_conversion() {
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

#[test]
fn serialize_deserialize_device() {
    let expected = Device {
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
    let serialized = serde_json::to_string(&expected).unwrap();
    let deserialized = serde_json::from_str::<Device>(&serialized).unwrap();
    assert_eq!(deserialized, expected);
}

#[test]
fn deserialize_device() {
    let expected = Device {
        id: "123".to_string(), // serialized as 'device_id'
        session_id: "123".to_string(),
        name: "test".to_string(),
        agent_version: "v1.0.0".to_string(),
        activated: true,
        status: DeviceStatus::Online,
        last_synced_at: Utc::now(),
        last_connected_at: Utc::now(),
        last_disconnected_at: Utc::now(),
    };
    let valid_input = json!({
        "device_id": expected.id,
        "session_id": expected.session_id,
        "name": expected.name,
        "agent_version": expected.agent_version,
        "activated": expected.activated,
        "status": expected.status,
        "last_synced_at": expected.last_synced_at,
        "last_connected_at": expected.last_connected_at,
        "last_disconnected_at": expected.last_disconnected_at,
    });
    let device: Device = serde_json::from_value(valid_input).unwrap();
    assert_eq!(device, expected);

    let empty_input = json!({});
    assert!(serde_json::from_value::<Device>(empty_input).is_err());

    let expected = Device {
        // required fields
        id: "123".to_string(), // serialized as 'device_id'
        // rest are defaults
        ..Default::default()
    };
    let valid_input = json!({
        "device_id": expected.id,
        "session_id": expected.session_id,
    });
    let device: Device = serde_json::from_value(valid_input).unwrap();
    assert_eq!(device, expected);

    // invalid JSON
    assert!(serde_json::from_str::<Device>("invalid-json").is_err());
}

#[test]
fn device_merge_empty() {
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
fn device_merge_all() {
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
