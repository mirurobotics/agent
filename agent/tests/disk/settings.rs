// internal crates
use miru_agent::disk::{Backend, MQTTBroker, Settings, TCPServer};
use miru_agent::logs::LogLevel;
use miru_agent::network::{BackendHost, MqttHost};
use miru_agent::server::DEFAULT_TCP_PORT;

// external crates
use serde_json::json;

#[test]
fn serialize_deserialize_settings() {
    let settings = Settings {
        log_level: LogLevel::Debug,
        enable_socket_server: false,
        enable_tcp_server: true,
        tcp_server: TCPServer { port: 51823 },
        enable_mqtt_worker: false,
        enable_poller: false,
        backend: Backend {
            host: BackendHost::new("staging.mirurobotics.com").unwrap(),
        },
        mqtt_broker: MQTTBroker {
            host: MqttHost::new("mqtt.staging.mirurobotics.com").unwrap(),
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
            host: BackendHost::new("staging.mirurobotics.com").unwrap(),
        },
        mqtt_broker: MQTTBroker {
            host: MqttHost::new("mqtt.staging.mirurobotics.com").unwrap(),
        },
        enable_socket_server: false,
        enable_tcp_server: true,
        tcp_server: TCPServer { port: 51823 },
        enable_mqtt_worker: false,
        enable_poller: false,
    };
    let valid_input = json!({
        "log_level": settings.log_level,
        "backend": settings.backend,
        "mqtt_broker": settings.mqtt_broker,
        "enable_socket_server": settings.enable_socket_server,
        "enable_tcp_server": settings.enable_tcp_server,
        "tcp_server": settings.tcp_server,
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
fn deserialize_ignores_legacy_is_persistent() {
    // settings files from before the removal of non-persistent mode still load
    let deserialized = serde_json::from_value::<Settings>(json!({"is_persistent": false})).unwrap();
    assert_eq!(deserialized, Settings::default());
}

#[test]
fn tcp_server_enabled_by_default_only_on_windows() {
    let settings = Settings::default();
    assert_eq!(settings.enable_tcp_server, cfg!(windows));
    assert_eq!(settings.tcp_server.port, DEFAULT_TCP_PORT);
}

#[test]
fn serialize_deserialize_backend() {
    let backend = Backend {
        host: BackendHost::new("staging.mirurobotics.com").unwrap(),
    };
    let serialized = serde_json::to_string(&backend).unwrap();
    let deserialized = serde_json::from_str::<Backend>(&serialized).unwrap();
    assert_eq!(deserialized, backend);
}

#[test]
fn backend_default_uses_backend_host_default() {
    let backend = Backend::default();
    assert_eq!(backend.host, BackendHost::default());
}

#[test]
fn deserialize_backend() {
    // valid deserialization
    let backend = Backend {
        host: BackendHost::new("staging.mirurobotics.com").unwrap(),
    };
    let valid_input = json!({
        "host": "staging.mirurobotics.com",
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
        host: MqttHost::new("mqtt.staging.mirurobotics.com").unwrap(),
    };
    let serialized = serde_json::to_string(&mqtt_broker).unwrap();
    let deserialized = serde_json::from_str::<MQTTBroker>(&serialized).unwrap();
    assert_eq!(deserialized, mqtt_broker);
}

#[test]
fn deserialize_mqtt_broker() {
    // valid deserialization
    let mqtt_broker = MQTTBroker {
        host: MqttHost::new("mqtt.staging.mirurobotics.com").unwrap(),
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
    // succeeds with the default host so the agent still talks to prod
    // (and the operator sees a warning in the logs). This case also serves
    // as the regression for legacy on-disk files: a JSON object containing
    // only the legacy `base_url` field is silently ignored as an unknown
    // key, the new `host` field is missing, and the warn-and-default path
    // produces `Backend::default()`.
    let input = json!({"host": "evilmirurobotics.com"});
    let backend = serde_json::from_value::<Backend>(input).unwrap();
    assert_eq!(backend, Backend::default());

    let legacy_input = json!({"base_url": "https://api.mirurobotics.com/agent/v1"});
    let legacy_backend = serde_json::from_value::<Backend>(legacy_input).unwrap();
    assert_eq!(legacy_backend, Backend::default());
}

#[test]
fn deserialize_backend_accepts_allowed_host() {
    let input = json!({"host": "api.mirurobotics.com"});
    let backend = serde_json::from_value::<Backend>(input).unwrap();
    assert_eq!(backend.host.as_str(), "api.mirurobotics.com");
}
