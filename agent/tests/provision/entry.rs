// internal crates
use crate::mocks::http_client::{self as mock, MockClient};
use backend_api::models::Device;
use miru_agent::crypt::base64;
use miru_agent::filesys::{self, PathExt};
use miru_agent::http::HTTPErr;
use miru_agent::provision::entry;
use miru_agent::provision::ProvisionErr;
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

        let device = entry::provision(
            &mock,
            &layout,
            &settings,
            &token,
            Some(device_name.to_string()),
        )
        .await
        .unwrap();

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

        let result = entry::provision(&mock, &layout, &settings, &token, None).await;

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
    async fn reprovision_overwrites_existing_storage() {
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
        entry::provision(&mock1, &layout, &settings, &token, Some("first".into()))
            .await
            .unwrap();

        let device_json: serde_json::Value =
            serde_json::from_str(&layout.device().read_string().await.unwrap()).unwrap();
        assert_eq!(device_json["name"], "first");

        let first_priv_bytes = auth_layout.private_key().read_string().await.unwrap();

        // second provision (re-provision)
        let mock2 = MockClient {
            provision_device_fn: Box::new(|| Ok(new_device(DEVICE_ID, "second"))),
            ..MockClient::default()
        };
        entry::provision(&mock2, &layout, &settings, &token, Some("second".into()))
            .await
            .unwrap();

        let device_json: serde_json::Value =
            serde_json::from_str(&layout.device().read_string().await.unwrap()).unwrap();
        assert_eq!(device_json["name"], "second");

        let second_priv_bytes = auth_layout.private_key().read_string().await.unwrap();
        assert_ne!(
            first_priv_bytes, second_priv_bytes,
            "expected fresh keypair on re-provision"
        );

        assert_eq!(mock2.call_count(mock::Call::ProvisionDevice), 1);
        assert!(!layout.temp_dir().exists(), "temp dir not cleaned");

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
        entry::provision(&mock_ok, &layout, &settings, &token, Some("initial".into()))
            .await
            .unwrap();

        // capture every persisted blob byte-for-byte
        let device_bytes = layout.device().read_string().await.unwrap();
        let settings_bytes = layout.settings().read_string().await.unwrap();
        let priv_key_bytes = auth_layout.private_key().read_string().await.unwrap();
        let pub_key_bytes = auth_layout.public_key().read_string().await.unwrap();
        let token_bytes = auth_layout.token().read_string().await.unwrap();

        // failing re-provision
        let mock_fail = MockClient {
            provision_device_fn: Box::new(|| {
                Err(HTTPErr::MockErr(miru_agent::http::errors::MockErr {
                    is_network_conn_err: true,
                }))
            }),
            ..MockClient::default()
        };
        let result = entry::provision(&mock_fail, &layout, &settings, &token, None).await;
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
}
