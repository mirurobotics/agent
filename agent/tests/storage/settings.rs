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

#[test]
fn deserialize_backend_falls_back_on_disallowed_host() {
    // Invalid hosts must not refuse the daemon's startup. Deserialization
    // succeeds with the default base_url so the agent still talks to prod
    // (and the operator sees a warning in the logs).
    let input = json!({"base_url": "https://evilmirurobotics.com"});
    let backend = serde_json::from_value::<Backend>(input).unwrap();
    assert_eq!(backend, Backend::default());
}

#[test]
fn deserialize_backend_accepts_allowed_host() {
    let input = json!({"base_url": "https://api.mirurobotics.com/agent/v1"});
    let backend = serde_json::from_value::<Backend>(input).unwrap();
    assert_eq!(backend.base_url, "https://api.mirurobotics.com/agent/v1");
}

#[test]
fn deserialize_backend_falls_back_on_http_non_loopback() {
    let input = json!({"base_url": "http://api.mirurobotics.com"});
    let backend = serde_json::from_value::<Backend>(input).unwrap();
    assert_eq!(backend, Backend::default());
}

#[test]
fn deserialize_mqtt_broker_falls_back_on_disallowed_host() {
    let input = json!({"host": "evilmirurobotics.com"});
    let mqtt_broker = serde_json::from_value::<MQTTBroker>(input).unwrap();
    assert_eq!(mqtt_broker, MQTTBroker::default());
}

#[test]
fn deserialize_mqtt_broker_accepts_allowed_host() {
    let input = json!({"host": "mqtt.mirurobotics.com"});
    let mqtt_broker = serde_json::from_value::<MQTTBroker>(input).unwrap();
    assert_eq!(mqtt_broker.host, "mqtt.mirurobotics.com");
}

#[test]
fn deserialize_settings_with_invalid_backend_url_falls_back() {
    let input = json!({
        "log_level": "info",
        "backend": {"base_url": "https://evilmirurobotics.com"},
        "mqtt_broker": {"host": "evilmirurobotics.com"},
        "is_persistent": true,
        "enable_socket_server": true,
        "enable_mqtt_worker": true,
        "enable_poller": true,
    });
    // The daemon must keep starting on a tampered settings file: invalid
    // backend/mqtt entries silently fall back to the defaults (with a
    // warning logged at deserialize time).
    let settings = serde_json::from_value::<Settings>(input).unwrap();
    assert_eq!(settings.backend, Backend::default());
    assert_eq!(settings.mqtt_broker, MQTTBroker::default());
}
