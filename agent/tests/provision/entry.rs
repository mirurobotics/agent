// internal crates
use crate::mocks::http_client::{self as mock, MockClient};
use backend_api::models::Device;
use miru_agent::crypt::base64;
use miru_agent::filesys::{self, PathExt};
use miru_agent::http::HTTPErr;
use miru_agent::provision::{self, errors::*};
use miru_agent::storage::{Layout, Settings};

// external crates
use serde_json::json;

const DEVICE_ID: &str = "75899aa4-b08a-4047-8526-880b1b832973";

fn new_jwt(device_id: &str) -> String {
    let payload = json!({
        "iss": "miru",
        "aud": "device",
        "exp": 9999999999_i64,
        "iat": 1700000000_i64,
        "sub": device_id
    })
    .to_string();
    format!(
        "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.{}.fakesig",
        base64::encode_string_url_safe_no_pad(&payload)
    )
}

fn new_device(id: &str, name: &str) -> Device {
    Device {
        id: id.to_string(),
        name: name.to_string(),
        session_id: "session-abc".to_string(),
        ..Device::default()
    }
}

pub mod provision_fn {
    use super::*;

    #[tokio::test]
    async fn success() {
        let token = new_jwt(DEVICE_ID);
        let device_name = "test-device";

        let root = filesys::Dir::create_temp_dir("provision-test")
            .await
            .unwrap();
        let layout = Layout::new(root.clone());
        let settings = Settings::default();

        let mock = MockClient {
            provision_device_fn: Box::new(move || Ok(new_device(DEVICE_ID, device_name))),
            ..MockClient::default()
        };

        let outcome = provision::provision(
            &mock,
            &layout,
            &settings,
            &token,
            Some(device_name.to_string()),
        )
        .await
        .unwrap();
        assert!(!outcome.is_provisioned);
        let device = outcome.device;

        assert_eq!(device.id, DEVICE_ID);
        assert_eq!(device.name, device_name);

        // device file was written
        let device_file = layout.device();
        assert!(device_file.exists(), "device.json missing");
        // device file contains the correct device ID
        let device_json: serde_json::Value =
            serde_json::from_str(&device_file.read_string().await.unwrap()).unwrap();
        assert_eq!(device_json["device_id"], DEVICE_ID);
        assert_eq!(device_json["name"], device_name);

        // settings file was written
        let settings_file = layout.settings();
        assert!(settings_file.exists(), "settings missing");

        // auth directory was created with keys and token
        let auth_layout = layout.auth();
        assert!(auth_layout.private_key().exists(), "private key missing");
        assert!(auth_layout.public_key().exists(), "public key missing");
        assert!(auth_layout.token().exists(), "token missing");

        // temp dir was cleaned up
        let temp_dir = layout.temp_dir();
        assert!(!temp_dir.exists(), "temp dir not deleted");

        // verify the mock was called
        assert_eq!(mock.call_count(mock::Call::ProvisionDevice), 1);

        root.delete().await.unwrap();
    }

    #[tokio::test]
    async fn http_error_aborts_provision() {
        let token = new_jwt(DEVICE_ID);

        let root = filesys::Dir::create_temp_dir("provision-test")
            .await
            .unwrap();
        let layout = Layout::new(root.clone());
        let settings = Settings::default();

        let mock = MockClient {
            provision_device_fn: Box::new(|| {
                Err(HTTPErr::MockErr(miru_agent::http::errors::MockErr {
                    is_network_conn_err: true,
                }))
            }),
            ..MockClient::default()
        };

        let result = provision::provision(&mock, &layout, &settings, &token, None).await;

        assert!(matches!(result, Err(ProvisionErr::HTTPErr(_))));

        let device_file = layout.device();
        assert!(
            !device_file.exists(),
            "device.json should not exist after failed provision"
        );

        // temp dir should still be cleaned up on error
        let temp_dir = layout.temp_dir();
        assert!(!temp_dir.exists(), "temp dir not deleted after error");

        root.delete().await.unwrap();
    }

    #[tokio::test]
    async fn provision_is_idempotent_on_second_call() {
        let token = new_jwt(DEVICE_ID);

        let root = filesys::Dir::create_temp_dir("provision-test")
            .await
            .unwrap();
        let layout = Layout::new(root.clone());
        let settings = Settings::default();
        let auth_layout = layout.auth();

        // first provision
        let mock1 = MockClient {
            provision_device_fn: Box::new(|| Ok(new_device(DEVICE_ID, "first"))),
            ..MockClient::default()
        };
        let outcome =
            provision::provision(&mock1, &layout, &settings, &token, Some("first".into()))
                .await
                .unwrap();
        assert!(!outcome.is_provisioned);

        let device_json: serde_json::Value =
            serde_json::from_str(&layout.device().read_string().await.unwrap()).unwrap();
        assert_eq!(device_json["name"], "first");

        let first_priv_bytes = auth_layout.private_key().read_string().await.unwrap();

        // second provision should now be a no-op because the box is already
        // activated AND the device file is parseable
        let mock2 = MockClient {
            provision_device_fn: Box::new(|| Ok(new_device(DEVICE_ID, "second"))),
            ..MockClient::default()
        };
        let outcome =
            provision::provision(&mock2, &layout, &settings, &token, Some("second".into()))
                .await
                .unwrap();
        assert!(outcome.is_provisioned);
        let device = outcome.device;

        // returned device matches the originally provisioned identity
        assert_eq!(device.id, DEVICE_ID);
        assert_eq!(device.name, "first");

        // device file is unchanged — still "first"
        let device_json: serde_json::Value =
            serde_json::from_str(&layout.device().read_string().await.unwrap()).unwrap();
        assert_eq!(device_json["name"], "first");

        // keys are byte-identical — no rotation occurred
        let second_priv_bytes = auth_layout.private_key().read_string().await.unwrap();
        assert_eq!(
            first_priv_bytes, second_priv_bytes,
            "expected idempotent provision to preserve keypair"
        );

        // backend was not called on the no-op path
        assert_eq!(mock2.call_count(mock::Call::ProvisionDevice), 0);
        assert!(!layout.temp_dir().exists(), "temp dir not cleaned");

        root.delete().await.unwrap();
    }

    #[tokio::test]
    async fn is_noop_when_already_activated() {
        let token = new_jwt(DEVICE_ID);

        let root = filesys::Dir::create_temp_dir("provision-test")
            .await
            .unwrap();
        let layout = Layout::new(root.clone());
        let settings = Settings::default();
        let auth_layout = layout.auth();

        // first provision lays down keys + device file
        let mock_ok = MockClient {
            provision_device_fn: Box::new(|| Ok(new_device(DEVICE_ID, "initial"))),
            ..MockClient::default()
        };
        let outcome =
            provision::provision(&mock_ok, &layout, &settings, &token, Some("initial".into()))
                .await
                .unwrap();
        assert!(!outcome.is_provisioned);

        let priv_bytes_before = auth_layout.private_key().read_string().await.unwrap();

        // poison-pill mock — any unexpected call would fail the test
        let mock_poison = MockClient {
            provision_device_fn: Box::new(|| {
                Err(HTTPErr::MockErr(miru_agent::http::errors::MockErr {
                    is_network_conn_err: true,
                }))
            }),
            ..MockClient::default()
        };

        let outcome = provision::provision(
            &mock_poison,
            &layout,
            &settings,
            &token,
            Some("ignored".into()),
        )
        .await
        .unwrap();
        assert!(outcome.is_provisioned);
        let device = outcome.device;

        // returned device matches the originally provisioned identity
        assert_eq!(device.id, DEVICE_ID);

        // backend was never called
        assert_eq!(mock_poison.call_count(mock::Call::ProvisionDevice), 0);

        // private key bytes are byte-identical — no rotation occurred
        let priv_bytes_after = auth_layout.private_key().read_string().await.unwrap();
        assert_eq!(priv_bytes_before, priv_bytes_after);

        root.delete().await.unwrap();
    }

    #[tokio::test]
    async fn falls_through_when_keys_missing() {
        use miru_agent::filesys::WriteOptions;

        let token = new_jwt(DEVICE_ID);

        let root = filesys::Dir::create_temp_dir("provision-test")
            .await
            .unwrap();
        let layout = Layout::new(root.clone());
        let settings = Settings::default();
        let auth_layout = layout.auth();

        // pre-create only device.json (no keys present)
        layout.root().create_if_absent().await.unwrap();
        let stub_device = serde_json::json!({
            "device_id": DEVICE_ID,
            "session_id": "sess",
            "name": "stub",
            "activated": true,
            "status": "online",
            "last_synced_at": "1970-01-01T00:00:00Z",
            "last_connected_at": "1970-01-01T00:00:00Z",
            "last_disconnected_at": "1970-01-01T00:00:00Z"
        });
        layout
            .device()
            .write_string(
                &serde_json::to_string(&stub_device).unwrap(),
                WriteOptions::OVERWRITE_ATOMIC,
            )
            .await
            .unwrap();

        // run a normal provision — must follow the full flow
        let mock = MockClient {
            provision_device_fn: Box::new(|| Ok(new_device(DEVICE_ID, "after-fallthrough"))),
            ..MockClient::default()
        };
        let outcome = provision::provision(
            &mock,
            &layout,
            &settings,
            &token,
            Some("after-fallthrough".into()),
        )
        .await
        .unwrap();
        assert!(!outcome.is_provisioned);
        let device = outcome.device;

        assert_eq!(device.name, "after-fallthrough");
        assert_eq!(mock.call_count(mock::Call::ProvisionDevice), 1);

        // keys were created
        assert!(auth_layout.private_key().exists());
        assert!(auth_layout.public_key().exists());

        root.delete().await.unwrap();
    }

    #[tokio::test]
    async fn falls_through_when_device_file_corrupt() {
        use miru_agent::filesys::WriteOptions;

        let token = new_jwt(DEVICE_ID);

        let root = filesys::Dir::create_temp_dir("provision-test")
            .await
            .unwrap();
        let layout = Layout::new(root.clone());
        let settings = Settings::default();
        let auth_layout = layout.auth();

        // first provision lays down keys + device file
        let mock_initial = MockClient {
            provision_device_fn: Box::new(|| Ok(new_device(DEVICE_ID, "initial"))),
            ..MockClient::default()
        };
        let outcome = provision::provision(
            &mock_initial,
            &layout,
            &settings,
            &token,
            Some("initial".into()),
        )
        .await
        .unwrap();
        assert!(!outcome.is_provisioned);

        // corrupt the device file
        layout
            .device()
            .write_string("not valid json{", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        // run provision again — short-circuit must fall through because the
        // device file does not parse, and the corrupt file must be overwritten
        let mock = MockClient {
            provision_device_fn: Box::new(|| Ok(new_device(DEVICE_ID, "recovered"))),
            ..MockClient::default()
        };
        let outcome =
            provision::provision(&mock, &layout, &settings, &token, Some("recovered".into()))
                .await
                .unwrap();
        assert!(!outcome.is_provisioned);
        let device = outcome.device;

        assert_eq!(device.name, "recovered");
        assert_eq!(mock.call_count(mock::Call::ProvisionDevice), 1);

        // device file was overwritten with valid content
        let device_json: serde_json::Value =
            serde_json::from_str(&layout.device().read_string().await.unwrap()).unwrap();
        assert_eq!(device_json["name"], "recovered");

        // keys still present (and rotated)
        assert!(auth_layout.private_key().exists());
        assert!(auth_layout.public_key().exists());

        root.delete().await.unwrap();
    }

    #[tokio::test]
    async fn http_error_on_reprovision_preserves_existing_storage() {
        let token = new_jwt(DEVICE_ID);

        let root = filesys::Dir::create_temp_dir("provision-test")
            .await
            .unwrap();
        let layout = Layout::new(root.clone());
        let settings = Settings::default();
        let auth_layout = layout.auth();

        // initial successful provision
        let mock_ok = MockClient {
            provision_device_fn: Box::new(|| Ok(new_device(DEVICE_ID, "initial"))),
            ..MockClient::default()
        };
        let outcome =
            provision::provision(&mock_ok, &layout, &settings, &token, Some("initial".into()))
                .await
                .unwrap();
        assert!(!outcome.is_provisioned);

        // capture every persisted blob byte-for-byte
        let device_bytes = layout.device().read_string().await.unwrap();
        let settings_bytes = layout.settings().read_string().await.unwrap();
        let priv_key_bytes = auth_layout.private_key().read_string().await.unwrap();
        let pub_key_bytes = auth_layout.public_key().read_string().await.unwrap();
        let token_bytes = auth_layout.token().read_string().await.unwrap();

        // failing re-provision — but provision is now idempotent, so it
        // short-circuits before reaching the mock and the failing mock is
        // never invoked. Storage is preserved either way.
        let mock_fail = MockClient {
            provision_device_fn: Box::new(|| {
                Err(HTTPErr::MockErr(miru_agent::http::errors::MockErr {
                    is_network_conn_err: true,
                }))
            }),
            ..MockClient::default()
        };
        let result = provision::provision(&mock_fail, &layout, &settings, &token, None).await;
        match &result {
            Ok(outcome) => assert!(outcome.is_provisioned),
            Err(_) => panic!("expected idempotent provision to short-circuit, got {result:?}"),
        }
        assert_eq!(mock_fail.call_count(mock::Call::ProvisionDevice), 0);

        // every captured blob is byte-identical
        assert_eq!(layout.device().read_string().await.unwrap(), device_bytes);
        assert_eq!(
            layout.settings().read_string().await.unwrap(),
            settings_bytes
        );
        assert_eq!(
            auth_layout.private_key().read_string().await.unwrap(),
            priv_key_bytes
        );
        assert_eq!(
            auth_layout.public_key().read_string().await.unwrap(),
            pub_key_bytes
        );
        assert_eq!(
            auth_layout.token().read_string().await.unwrap(),
            token_bytes
        );

        assert!(
            !layout.temp_dir().exists(),
            "temp dir not deleted after error"
        );

        root.delete().await.unwrap();
    }
}

pub mod reprovision_fn {
    use super::*;

    #[tokio::test]
    async fn success() {
        let token = new_jwt(DEVICE_ID);

        let root = filesys::Dir::create_temp_dir("reprovision-test")
            .await
            .unwrap();
        let layout = Layout::new(root.clone());
        let settings = Settings::default();

        let mock = MockClient {
            reprovision_device_fn: Box::new(move || Ok(new_device(DEVICE_ID, "after-reprovision"))),
            ..MockClient::default()
        };

        let device = provision::reprovision(&mock, &layout, &settings, &token)
            .await
            .unwrap();

        assert_eq!(device.id, DEVICE_ID);
        assert_eq!(device.name, "after-reprovision");

        // device file was written
        let device_file = layout.device();
        assert!(device_file.exists(), "device.json missing");
        let device_json: serde_json::Value =
            serde_json::from_str(&device_file.read_string().await.unwrap()).unwrap();
        assert_eq!(device_json["device_id"], DEVICE_ID);
        assert_eq!(device_json["name"], "after-reprovision");

        // settings file was written
        let settings_file = layout.settings();
        assert!(settings_file.exists(), "settings missing");

        // auth dir was created with keys and token
        let auth_layout = layout.auth();
        assert!(auth_layout.private_key().exists(), "private key missing");
        assert!(auth_layout.public_key().exists(), "public key missing");
        assert!(auth_layout.token().exists(), "token missing");

        // temp dir was cleaned up
        assert!(!layout.temp_dir().exists(), "temp dir not deleted");

        // verify the mock was called and only the reprovision endpoint was hit
        assert_eq!(mock.call_count(mock::Call::ReprovisionDevice), 1);
        assert_eq!(mock.call_count(mock::Call::ProvisionDevice), 0);

        // load-bearing wire-format invariant: the captured POST body has
        // public_key_pem and agent_version but NO name field
        let requests = mock.requests();
        let captured = requests
            .iter()
            .find(|r| r.call == mock::Call::ReprovisionDevice)
            .expect("expected a reprovision request");
        let body = captured.body.as_ref().expect("expected body to be present");
        let body_value: serde_json::Value = serde_json::from_str(body).unwrap();
        assert!(
            body_value.get("public_key_pem").is_some(),
            "expected public_key_pem in body, got {body}"
        );
        assert!(
            body_value.get("agent_version").is_some(),
            "expected agent_version in body, got {body}"
        );
        assert!(
            body_value.get("name").is_none(),
            "expected NO name field in body, got {body}"
        );

        root.delete().await.unwrap();
    }

    #[tokio::test]
    async fn http_error_preserves_existing_storage() {
        let token = new_jwt(DEVICE_ID);

        let root = filesys::Dir::create_temp_dir("reprovision-test")
            .await
            .unwrap();
        let layout = Layout::new(root.clone());
        let settings = Settings::default();
        let auth_layout = layout.auth();

        // initial successful provision
        let mock_ok = MockClient {
            provision_device_fn: Box::new(|| Ok(new_device(DEVICE_ID, "initial"))),
            ..MockClient::default()
        };
        provision::provision(&mock_ok, &layout, &settings, &token, Some("initial".into()))
            .await
            .unwrap();

        // capture every persisted blob byte-for-byte
        let device_bytes = layout.device().read_string().await.unwrap();
        let settings_bytes = layout.settings().read_string().await.unwrap();
        let priv_key_bytes = auth_layout.private_key().read_string().await.unwrap();
        let pub_key_bytes = auth_layout.public_key().read_string().await.unwrap();
        let token_bytes = auth_layout.token().read_string().await.unwrap();

        // failing reprovision
        let mock_fail = MockClient {
            reprovision_device_fn: Box::new(|| {
                Err(HTTPErr::MockErr(miru_agent::http::errors::MockErr {
                    is_network_conn_err: true,
                }))
            }),
            ..MockClient::default()
        };
        let result = provision::reprovision(&mock_fail, &layout, &settings, &token).await;
        assert!(matches!(result, Err(ProvisionErr::HTTPErr(_))));

        // every captured blob is byte-identical
        assert_eq!(layout.device().read_string().await.unwrap(), device_bytes);
        assert_eq!(
            layout.settings().read_string().await.unwrap(),
            settings_bytes
        );
        assert_eq!(
            auth_layout.private_key().read_string().await.unwrap(),
            priv_key_bytes
        );
        assert_eq!(
            auth_layout.public_key().read_string().await.unwrap(),
            pub_key_bytes
        );
        assert_eq!(
            auth_layout.token().read_string().await.unwrap(),
            token_bytes
        );

        assert!(
            !layout.temp_dir().exists(),
            "temp dir not deleted after error"
        );

        root.delete().await.unwrap();
    }

    #[tokio::test]
    async fn rotates_keypair() {
        let token = new_jwt(DEVICE_ID);

        let root = filesys::Dir::create_temp_dir("reprovision-test")
            .await
            .unwrap();
        let layout = Layout::new(root.clone());
        let settings = Settings::default();
        let auth_layout = layout.auth();

        // initial successful provision
        let mock_ok = MockClient {
            provision_device_fn: Box::new(|| Ok(new_device(DEVICE_ID, "initial"))),
            ..MockClient::default()
        };
        provision::provision(&mock_ok, &layout, &settings, &token, Some("initial".into()))
            .await
            .unwrap();

        let priv_bytes_before = auth_layout.private_key().read_string().await.unwrap();

        // reprovision must always rotate keys
        let mock_reprovision = MockClient {
            reprovision_device_fn: Box::new(|| Ok(new_device(DEVICE_ID, "after-reprovision"))),
            ..MockClient::default()
        };
        provision::reprovision(&mock_reprovision, &layout, &settings, &token)
            .await
            .unwrap();

        let priv_bytes_after = auth_layout.private_key().read_string().await.unwrap();
        assert_ne!(
            priv_bytes_before, priv_bytes_after,
            "expected fresh keypair after reprovision"
        );

        root.delete().await.unwrap();
    }
}
