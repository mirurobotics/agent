// internal crates
use super::shared::{
    assert_temp_dir_cleaned, mock_failing_provision, mock_ok_provision, validate_storage, Env,
    StorageSnapshot, DEVICE_ID,
};
use crate::mocks::http_client as mock;
use crate::test_utils::filesys::files as test_files;
use miru_agent::filesys::{dirs, files, PathExt};
use miru_agent::provisioning::{errors::*, provision};

pub mod provision_fn {
    use super::*;

    #[tokio::test]
    async fn success() {
        let env = Env::new("provision-test").await;
        let mock = mock_ok_provision("test-device");

        let device = provision::provision(
            &mock,
            &env.layout,
            &env.settings,
            &env.token,
            Some("test-device".into()),
        )
        .await
        .unwrap();

        assert_eq!(device.name, "test-device");
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
        assert_temp_dir_cleaned(&env.layout).await;

        env.cleanup().await;
    }

    #[tokio::test]
    async fn fails_when_already_provisioned() {
        let env = Env::new("provision-test").await;
        env.seed_provision("first").await;
        let snapshot = StorageSnapshot::capture(&env.layout).await;

        // the backend would accept a second provision, so only the local check
        // can stop it
        let mock = mock_ok_provision("second");
        let err = provision::provision(
            &mock,
            &env.layout,
            &env.settings,
            &env.token,
            Some("second".into()),
        )
        .await
        .unwrap_err();

        let ProvisionErr::AlreadyProvisionedErr(already) = &err else {
            panic!("expected AlreadyProvisionedErr, got {err:?}");
        };
        assert_eq!(already.device_name, "first");
        assert!(err.to_string().contains("miru-agent reprovision"));
        assert_eq!(mock.call_count(mock::Call::ProvisionDevice), 0);
        snapshot.assert_unchanged(&env.layout).await;

        env.cleanup().await;
    }

    #[tokio::test]
    async fn falls_through_when_keys_missing() {
        let env = Env::new("provision-test").await;

        // pre-create only device.json — no keys, so assert_activated fails
        // and the short-circuit doesn't trigger
        dirs::create_if_absent(&env.layout.root()).await.unwrap();
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
        test_files::seed(
            &env.layout.device(),
            &serde_json::to_string(&stub_device).unwrap(),
        )
        .await;

        let mock = mock_ok_provision("after-fallthrough");
        let device = provision::provision(
            &mock,
            &env.layout,
            &env.settings,
            &env.token,
            Some("after-fallthrough".into()),
        )
        .await
        .unwrap();

        assert_eq!(device.name, "after-fallthrough");
        assert_eq!(mock.call_count(mock::Call::ProvisionDevice), 1);
        validate_storage(&env.layout, "after-fallthrough").await;

        env.cleanup().await;
    }

    #[tokio::test]
    async fn http_error_preserves_existing_storage() {
        let env = Env::new("provision-test").await;
        env.seed_provision("initial").await;

        // delete the public key to trigger provisioning
        files::delete(&env.layout.auth().public_key())
            .await
            .unwrap();

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
        assert_temp_dir_cleaned(&env.layout).await;

        env.cleanup().await;
    }

    #[tokio::test]
    async fn already_provisioned_with_unparseable_device_json_reports_unknown_name() {
        let env = Env::new("provision-test").await;
        env.seed_provision("initial").await;

        // corrupt device.json so read_json fails, but keep the keys intact so
        // assert_activated still succeeds
        test_files::seed(&env.layout.device(), "not valid json").await;

        let mock = mock_failing_provision();
        let err = provision::provision(&mock, &env.layout, &env.settings, &env.token, None)
            .await
            .unwrap_err();

        let ProvisionErr::AlreadyProvisionedErr(already) = &err else {
            panic!("expected AlreadyProvisionedErr, got {err:?}");
        };
        assert_eq!(already.device_name, "unknown");
        assert_eq!(mock.call_count(mock::Call::ProvisionDevice), 0);

        env.cleanup().await;
    }
}
