// internal crates
use miru_agent::logs::LogLevel;
use miru_agent::storage::{Backend, MQTTBroker, Settings};

// external crates
use serde_json::json;

#[test]
fn serialize_deserialize_settings() {
    let settings = Settings {
        log_level: LogLevel::Debug,
        is_persistent: false,
        enable_socket_server: false,
        enable_mqtt_worker: false,
        enable_poller: false,
        backend: Backend {
            base_url: "https://staging.mirurobotics.com/agent/v1".to_string(),
        },
        mqtt_broker: MQTTBroker {
            host: "mqtt.staging.mirurobotics.com".to_string(),
        },
    };
    let serialized = serde_json::to_string(&settings).unwrap();
    let deserialized = serde_json::from_str::<Settings>(&serialized).unwrap();
    assert_eq!(deserialized, settings);
}

#[test]
fn deserialize_settings() {
    // valid deserialization
    let settings = Settings {
        log_level: LogLevel::Debug,
        backend: Backend {
            base_url: "https://staging.mirurobotics.com/agent/v1".to_string(),
        },
        mqtt_broker: MQTTBroker {
            host: "mqtt.staging.mirurobotics.com".to_string(),
        },
        is_persistent: false,
        enable_socket_server: false,
        enable_mqtt_worker: false,
        enable_poller: false,
    };
    let valid_input = json!({
        "log_level": settings.log_level,
        "backend": settings.backend,
        "mqtt_broker": settings.mqtt_broker,
        "is_persistent": settings.is_persistent,
        "enable_socket_server": settings.enable_socket_server,
        "enable_mqtt_worker": settings.enable_mqtt_worker,
        "enable_poller": settings.enable_poller,
    });
    let deserialized = serde_json::from_value::<Settings>(valid_input).unwrap();
    assert_eq!(deserialized, settings);

    // no fields are required so we can't test that w/out required fields throws error

    // exclude default fields
    let settings = Settings::default();
    let valid_input = json!({});
    let deserialized = serde_json::from_value::<Settings>(valid_input).unwrap();
    assert_eq!(deserialized, settings);

    // invalid JSON
    assert!(serde_json::from_str::<Settings>("invalid-json").is_err());
}

#[test]
fn serialize_deserialize_backend() {
    let backend = Backend {
        base_url: "https://staging.mirurobotics.com/agent/v1".to_string(),
    };
    let serialized = serde_json::to_string(&backend).unwrap();
    let deserialized = serde_json::from_str::<Backend>(&serialized).unwrap();
    assert_eq!(deserialized, backend);
}

#[test]
fn deserialize_backend() {
    // valid deserialization
    let backend = Backend {
        base_url: "https://staging.mirurobotics.com/agent/v1".to_string(),
    };
    let valid_input = json!({
        "base_url": backend.base_url,
    });
    let deserialized = serde_json::from_value::<Backend>(valid_input).unwrap();
    assert_eq!(deserialized, backend);

    // no fields are required so we can't test that w/out required fields throws error

    // exclude default fields
    let backend = Backend::default();
    let valid_input = json!({});
    let deserialized = serde_json::from_value::<Backend>(valid_input).unwrap();
    assert_eq!(deserialized, backend);

    // invalid JSON
    assert!(serde_json::from_str::<Backend>("invalid-json").is_err());
}

#[test]
fn serialize_deserialize_mqtt_broker() {
    let mqtt_broker = MQTTBroker {
        host: "mqtt.staging.mirurobotics.com".to_string(),
    };
    let serialized = serde_json::to_string(&mqtt_broker).unwrap();
    let deserialized = serde_json::from_str::<MQTTBroker>(&serialized).unwrap();
    assert_eq!(deserialized, mqtt_broker);
}

#[test]
fn deserialize_mqtt_broker() {
    // valid deserialization
    let mqtt_broker = MQTTBroker {
        host: "mqtt.staging.mirurobotics.com".to_string(),
    };
    let valid_input = json!({
        "host": mqtt_broker.host,
    });
    let deserialized = serde_json::from_value::<MQTTBroker>(valid_input).unwrap();
    assert_eq!(deserialized, mqtt_broker);

    // no fields are required so we can't test that w/out required fields throws error

    // exclude default fields
    let mqtt_broker = MQTTBroker::default();
    let valid_input = json!({});
    let deserialized = serde_json::from_value::<MQTTBroker>(valid_input).unwrap();
    assert_eq!(deserialized, mqtt_broker);

    // invalid JSON
    assert!(serde_json::from_str::<MQTTBroker>("invalid-json").is_err());
}
