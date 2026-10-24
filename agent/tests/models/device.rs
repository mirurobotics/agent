// internal crates
use device_api::models as agent_server;
use miru_agent::models::device::Updates;
use miru_agent::models::Patch;
use miru_agent::models::{Device, DeviceStatus};

// external crates
use chrono::{DateTime, Duration, Utc};
use serde_json::json;

// harness
use crate::models::harnesses::{
    serde_tests, status_enum_tests, ModelFixture, OptionalField, RequiredField, StatusCase,
    StatusEnumFixture,
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
        activated: false,
        status: DeviceStatus::Offline,
        last_synced_at: DateTime::<Utc>::UNIX_EPOCH,
        last_connected_at: DateTime::<Utc>::UNIX_EPOCH,
        last_disconnected_at: DateTime::<Utc>::UNIX_EPOCH,
    };
    assert_eq!(device, expected);
}

// ─── status enum tests ───────────────────────────────────────────────────────

#[test]
fn status_default() {
    let status = DeviceStatus::default();
    assert_eq!(status, DeviceStatus::Offline);
}

impl StatusEnumFixture for DeviceStatus {
    type Backend = ();
    type Agent = agent_server::DeviceStatus;

    fn cases() -> Vec<StatusCase<Self>> {
        vec![
            StatusCase {
                variant: DeviceStatus::Online,
                wire: "online",
                backend: (),
                agent: agent_server::DeviceStatus::DEVICE_STATUS_ONLINE,
            },
            StatusCase {
                variant: DeviceStatus::Offline,
                wire: "offline",
                backend: (),
                agent: agent_server::DeviceStatus::DEVICE_STATUS_OFFLINE,
            },
        ]
    }
}

status_enum_tests!(DeviceStatus, agent);

// ─── merge tests ──────────────────────────────────────────────────────────────

#[test]
fn merge_empty() {
    let initial = Device {
        id: "123".to_string(),
        session_id: "123".to_string(),
        name: "test".to_string(),
        activated: true,
        status: DeviceStatus::Online,
        last_synced_at: Utc::now(),
        last_connected_at: Utc::now(),
        last_disconnected_at: Utc::now(),
    };
    let updates = Updates::empty();
    let expected = initial.clone();
    let mut actual = initial.clone();
    actual.patch(updates);
    assert_eq!(expected, actual);
}

#[test]
fn merge_all() {
    let initial = Device {
        id: "123".to_string(),
        session_id: "123".to_string(),
        name: "test".to_string(),
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
        status: Some(DeviceStatus::Offline),
        last_synced_at: Some(Utc::now() + Duration::days(1)),
        last_connected_at: Some(Utc::now() + Duration::days(1)),
        last_disconnected_at: Some(Utc::now() + Duration::days(1)),
    };
    let expected = Device {
        id: updates.id.clone().unwrap(),
        session_id: initial.session_id.clone(),
        name: updates.name.clone().unwrap(),
        activated: updates.activated.unwrap(),
        status: updates.status.clone().unwrap(),
        last_synced_at: updates.last_synced_at.unwrap(),
        last_connected_at: updates.last_connected_at.unwrap(),
        last_disconnected_at: updates.last_disconnected_at.unwrap(),
    };
    let mut actual = initial.clone();
    actual.patch(updates);
    assert_eq!(expected, actual);
}

// ─── updates tests ────────────────────────────────────────────────────────────
#[test]
fn updates_empty() {
    let actual = Updates::empty();
    let expected = Updates {
        id: None,
        name: None,
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
    let at = "2026-10-21T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
    let actual = Updates::disconnected(at);

    let expected = Updates {
        status: Some(DeviceStatus::Offline),
        last_disconnected_at: Some(at),
        ..Updates::empty()
    };

    assert_eq!(actual, expected);
}

#[test]
fn updates_connected() {
    let at = "2026-10-21T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
    let actual = Updates::connected(at);

    let expected = Updates {
        status: Some(DeviceStatus::Online),
        last_connected_at: Some(at),
        ..Updates::empty()
    };
    assert_eq!(actual, expected);
}
