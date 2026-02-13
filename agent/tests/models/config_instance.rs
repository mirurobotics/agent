// internal crates
use miru_agent::models::config_instance::ConfigInstance;
use openapi_client::models::ConfigInstance as BackendConfigInstance;

// external crates
use chrono::{DateTime, Utc};

#[test]
fn config_instance_from_backend() {
    let now = Utc::now();
    let backend_instance = BackendConfigInstance {
        object: openapi_client::models::config_instance::Object::ConfigInstance,
        id: "cfg_inst_123".to_string(),
        config_type_name: "motion-control".to_string(),
        filepath: "v1/motion-control.json".to_string(),
        created_at: now.to_rfc3339(),
        config_schema_id: "schema_123".to_string(),
        config_type_id: "type_123".to_string(),
        config_type: None,
        content: None,
    };

    let config_instance = ConfigInstance::from_backend(backend_instance);

    assert_eq!(config_instance.id, "cfg_inst_123");
    assert_eq!(config_instance.config_type_name, "motion-control");
    assert_eq!(config_instance.filepath, "v1/motion-control.json");
    assert_eq!(config_instance.config_schema_id, "schema_123");
    assert_eq!(config_instance.config_type_id, "type_123");
    // Verify created_at was parsed correctly
    assert!(config_instance.created_at > DateTime::<Utc>::UNIX_EPOCH);
}

#[test]
fn config_instance_serialize_deserialize() {
    let config_instance = ConfigInstance {
        id: "cfg_inst_123".to_string(),
        config_type_name: "motion-control".to_string(),
        filepath: "v1/motion-control.json".to_string(),
        created_at: Utc::now(),
        config_schema_id: "schema_123".to_string(),
        config_type_id: "type_123".to_string(),
    };

    let serialized = serde_json::to_string(&config_instance).unwrap();
    let deserialized: ConfigInstance = serde_json::from_str(&serialized).unwrap();

    assert_eq!(deserialized.id, config_instance.id);
    assert_eq!(
        deserialized.config_type_name,
        config_instance.config_type_name
    );
    assert_eq!(deserialized.filepath, config_instance.filepath);
    assert_eq!(
        deserialized.config_schema_id,
        config_instance.config_schema_id
    );
    assert_eq!(deserialized.config_type_id, config_instance.config_type_id);
    // DateTime comparison with small tolerance
    let time_diff = (deserialized.created_at - config_instance.created_at)
        .num_seconds()
        .abs();
    assert!(
        time_diff < 1,
        "Time difference should be less than 1 second"
    );
}
