// internal crates
use super::shared::{
    mock_failing_provision, mock_ok_provision, validate_storage, Env, StorageSnapshot, DEVICE_ID,
};
use crate::mocks::http_client as mock;
use miru_agent::filesys::{PathExt, WriteOptions};
use miru_agent::provisioning::{errors::*, provision};

pub mod provision_fn {
    use super::*;

    #[tokio::test]
    async fn success() {
        let env = Env::new("provision-test").await;
        let mock = mock_ok_provision("test-device");

        let outcome = provision::provision(
            &mock,
            &env.layout,
            &env.settings,
            &env.token,
            Some("test-device".into()),
        )
        .await
        .unwrap();

        assert!(!outcome.already_provisioned);
        assert_eq!(outcome.device_name, "test-device");
        assert_eq!(mock.call_count(mock::Call::ProvisionDevice), 1);
        validate_storage(&env.layout, "test-device").await;

        env.cleanup().await;
    }

    #[tokio::test]
    async fn http_error_aborts_provision() {
        let env = Env::new("provision-test").await;
        let mock = mock_failing_provision();

        let result =
            provision::provision(&mock, &env.layout, &env.settings, &env.token, None).await;

        assert!(matches!(result, Err(ProvisionErr::HTTPErr(_))));
        assert!(
            !env.layout.device().exists(),
            "device.json should not exist"
        );
        assert!(!env.layout.temp_dir().exists(), "temp dir not cleaned");

        env.cleanup().await;
    }

    #[tokio::test]
    async fn provision_is_idempotent_on_second_call() {
        let env = Env::new("provision-test").await;
        env.seed_provision("first").await;
        let snapshot = StorageSnapshot::capture(&env.layout).await;

        // second call should short-circuit, not call the backend, and leave
        // every persisted blob byte-identical
        let mock = mock_ok_provision("second");
        let outcome = provision::provision(
            &mock,
            &env.layout,
            &env.settings,
            &env.token,
            Some("second".into()),
        )
        .await
        .unwrap();

        assert!(outcome.already_provisioned);
        assert_eq!(outcome.device_name, "first");
        assert_eq!(mock.call_count(mock::Call::ProvisionDevice), 0);
        snapshot.assert_unchanged(&env.layout).await;

        env.cleanup().await;
    }

    #[tokio::test]
    async fn is_noop_when_already_activated() {
        let env = Env::new("provision-test").await;
        env.seed_provision("initial").await;
        let snapshot = StorageSnapshot::capture(&env.layout).await;

        // a poison-pill mock — any call would fail the test
        let mock = mock_failing_provision();
        let outcome = provision::provision(
            &mock,
            &env.layout,
            &env.settings,
            &env.token,
            Some("ignored".into()),
        )
        .await
        .unwrap();

        assert!(outcome.already_provisioned);
        assert_eq!(outcome.device_name, "initial");
        assert_eq!(mock.call_count(mock::Call::ProvisionDevice), 0);
        snapshot.assert_unchanged(&env.layout).await;

        env.cleanup().await;
    }

    #[tokio::test]
    async fn falls_through_when_keys_missing() {
        let env = Env::new("provision-test").await;

        // pre-create only device.json — no keys, so assert_activated fails
        // and the short-circuit doesn't trigger
        env.layout.root().create_if_absent().await.unwrap();
        let stub_device = serde_json::json!({
            "device_id": DEVICE_ID,
            "session_id": "sess",
            "name": "stub",
            "activated": true,
            "status": "online",
            "last_synced_at": "1970-01-01T00:00:00Z",
            "last_connected_at": "1970-01-01T00:00:00Z",
            "last_disconnected_at": "1970-01-01T00:00:00Z",
        });
        env.layout
            .device()
            .write_string(
                &serde_json::to_string(&stub_device).unwrap(),
                WriteOptions::OVERWRITE_ATOMIC,
            )
            .await
            .unwrap();

        let mock = mock_ok_provision("after-fallthrough");
        let outcome = provision::provision(
            &mock,
            &env.layout,
            &env.settings,
            &env.token,
            Some("after-fallthrough".into()),
        )
        .await
        .unwrap();

        assert!(!outcome.already_provisioned);
        assert_eq!(outcome.device_name, "after-fallthrough");
        assert_eq!(mock.call_count(mock::Call::ProvisionDevice), 1);
        validate_storage(&env.layout, "after-fallthrough").await;

        env.cleanup().await;
    }

    #[tokio::test]
    async fn http_error_preserves_existing_storage() {
        let env = Env::new("provision-test").await;
        env.seed_provision("initial").await;

        // delete the public key to trigger provisioning
        env.layout.auth().public_key().delete().await.unwrap();

        let snapshot = StorageSnapshot::capture(&env.layout).await;

        // a failing mock — but provision short-circuits before reaching it
        // because the device is already activated, so storage is preserved
        let mock = mock_failing_provision();
        let err = provision::provision(&mock, &env.layout, &env.settings, &env.token, None)
            .await
            .unwrap_err();
        assert!(matches!(err, ProvisionErr::HTTPErr(_)));

        assert_eq!(mock.call_count(mock::Call::ProvisionDevice), 1);
        snapshot.assert_unchanged(&env.layout).await;
        assert!(!env.layout.temp_dir().exists(), "temp dir not cleaned");

        env.cleanup().await;
    }

    #[tokio::test]
    async fn short_circuit_with_unparseable_device_json_returns_unknown_name() {
        let env = Env::new("provision-test").await;
        env.seed_provision("initial").await;

        // corrupt device.json so read_json fails, but keep the keys intact so
        // assert_activated still succeeds and the short-circuit branch runs
        env.layout
            .device()
            .write_string("not valid json", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        // a failing mock — short-circuit must still fire even when device.json
        // is unreadable, so the backend should never be called
        let mock = mock_failing_provision();
        let outcome = provision::provision(&mock, &env.layout, &env.settings, &env.token, None)
            .await
            .unwrap();

        assert!(outcome.already_provisioned);
        assert_eq!(outcome.device_name, "unknown");
        assert_eq!(mock.call_count(mock::Call::ProvisionDevice), 0);

        env.cleanup().await;
    }
}
