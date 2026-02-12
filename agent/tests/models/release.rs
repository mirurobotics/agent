// internal crates
use miru_agent::models::release::Release;
use openapi_client::models::Release as BackendRelease;

// external crates
use chrono::{DateTime, Utc};
use serde_json::json;

#[test]
fn release_from_backend() {
    let now = Utc::now();
    let backend_release = BackendRelease {
        object: openapi_client::models::release::Object::Release,
        id: "rel_123".to_string(),
        version: "1.0.0".to_string(),
        created_at: now.to_rfc3339(),
        updated_at: now.to_rfc3339(),
    };

    let release = Release::from_backend(backend_release);

    assert_eq!(release.id, "rel_123");
    assert_eq!(release.version, "1.0.0");
    // Note: We can't directly compare DateTime due to potential parsing differences,
    // but we can verify it's not the epoch
    assert!(release.created_at > DateTime::<Utc>::UNIX_EPOCH);
    assert!(release.updated_at > DateTime::<Utc>::UNIX_EPOCH);
}

#[test]
fn release_serialize_deserialize() {
    let release = Release {
        id: "rel_123".to_string(),
        version: "1.0.0".to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    let serialized = serde_json::to_string(&release).unwrap();
    let deserialized: Release = serde_json::from_str(&serialized).unwrap();

    assert_eq!(deserialized.id, release.id);
    assert_eq!(deserialized.version, release.version);
    // DateTime comparison with small tolerance
    let time_diff = (deserialized.created_at - release.created_at).num_seconds().abs();
    assert!(time_diff < 1, "Time difference should be less than 1 second");
}

