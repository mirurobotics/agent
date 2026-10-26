// internal crates
use super::shared::{
    assert_temp_dir_cleaned, mock_failing_provision, mock_ok_provision, validate_storage, Env,
    StorageSnapshot, DEVICE_ID,
};
use crate::mocks::http_client::{self as mock, MockClient};
use crate::test_utils::filesys::files as test_files;
use miru_agent::authn::AuthnErr;
use miru_agent::filesys::{dirs, files, PathExt};
use miru_agent::http::{errors::RequestFailed, request::Params, HTTPErr};
use miru_agent::provisioning::{errors::*, provision};

/// MockClient whose `/devices/token` fails with `status`.
fn mock_token_status(status: u16) -> MockClient {
    MockClient {
        issue_device_token_fn: Box::new(move || {
            Err(HTTPErr::RequestFailed(RequestFailed {
                request: Params::post("http://mock/devices/token", String::new())
                    .meta()
                    .unwrap(),
                status: reqwest::StatusCode::from_u16(status).unwrap(),
                error: None,
                trace: miru_agent::trace!(),
            }))
        }),
        ..MockClient::default()
    }
}

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
        assert_temp_dir_cleaned(&env.layout).await;

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

        // a poison-pill provision mock — reaching /devices/provision would fail the test
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
        assert_eq!(mock.call_count(mock::Call::IssueDeviceToken), 1);
        snapshot.assert_unchanged(&env.layout).await;

        env.cleanup().await;
    }

    #[tokio::test]
    async fn fails_when_miru_rejects_existing_device() {
        let env = Env::new("provision-test").await;
        env.seed_provision("initial").await;
        let snapshot = StorageSnapshot::capture(&env.layout).await;

        // e.g. the device was archived in Miru after this machine was wiped
        let mock = mock_token_status(400);
        let err = provision::provision(&mock, &env.layout, &env.settings, &env.token, None)
            .await
            .unwrap_err();

        let ProvisionErr::DeviceRejectedErr(rejected) = &err else {
            panic!("expected DeviceRejectedErr, got {err:?}");
        };
        assert_eq!(rejected.device_name, "initial");
        assert!(err.to_string().contains("miru-agent reprovision"));
        assert_eq!(mock.call_count(mock::Call::ProvisionDevice), 0);
        snapshot.assert_unchanged(&env.layout).await;

        env.cleanup().await;
    }

    #[tokio::test]
    async fn transient_error_verifying_existing_device_is_not_a_rejection() {
        for status in [408, 429, 500] {
            let env = Env::new("provision-test").await;
            env.seed_provision("initial").await;

            let mock = mock_token_status(status);
            let err = provision::provision(&mock, &env.layout, &env.settings, &env.token, None)
                .await
                .unwrap_err();

            assert!(
                matches!(err, ProvisionErr::AuthnErr(AuthnErr::HTTPErr(_))),
                "status {status}: {err:?}"
            );
            assert_eq!(mock.call_count(mock::Call::ProvisionDevice), 0);

            env.cleanup().await;
        }
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
    async fn short_circuit_with_unparseable_device_json_returns_unknown_name() {
        let env = Env::new("provision-test").await;
        env.seed_provision("initial").await;

        // corrupt device.json so read_json fails, but keep the keys intact so
        // assert_activated still succeeds and the short-circuit branch runs
        test_files::seed(&env.layout.device(), "not valid json").await;

        // a failing mock — short-circuit must still fire even when device.json
        // is unreadable, so /devices/provision should never be called
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
