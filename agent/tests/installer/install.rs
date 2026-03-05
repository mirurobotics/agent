// internal crates
use crate::http::mock;
use crate::http::mock::MockClient;
use backend_api::models::Device;
use miru_agent::crypt::base64;
use miru_agent::filesys::{self, PathExt};
use miru_agent::http::HTTPErr;
use miru_agent::installer::install;
use miru_agent::installer::InstallErr;
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

pub mod install_fn {
    use super::*;

    #[tokio::test]
    async fn success() {
        let token = new_jwt(DEVICE_ID);
        let device_name = "test-device";

        let root = filesys::Dir::create_temp_dir("install-test").await.unwrap();
        let layout = Layout::new(root.clone());
        let settings = Settings::default();

        let mock = MockClient {
            activate_device_fn: Box::new(move || Ok(new_device(DEVICE_ID, device_name))),
            ..MockClient::default()
        };

        let device = install::install(
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
        assert_eq!(mock.call_count(mock::Call::ActivateDevice), 1);

        root.delete().await.unwrap();
    }

    #[tokio::test]
    async fn http_error_stops_install() {
        let token = new_jwt(DEVICE_ID);

        let root = filesys::Dir::create_temp_dir("install-test").await.unwrap();
        let layout = Layout::new(root.clone());
        let settings = Settings::default();

        let mock = MockClient {
            activate_device_fn: Box::new(|| {
                Err(HTTPErr::MockErr(miru_agent::http::errors::MockErr {
                    is_network_conn_err: true,
                }))
            }),
            ..MockClient::default()
        };

        let result = install::install(&mock, &layout, &settings, &token, None).await;

        assert!(matches!(result, Err(InstallErr::HTTPErr(_))));

        let device_file = layout.device();
        assert!(
            !device_file.exists(),
            "device.json should not exist after failed install"
        );

        // temp dir should still be cleaned up on error
        let temp_dir = layout.temp_dir();
        assert!(!temp_dir.exists(), "temp dir not deleted after error");

        root.delete().await.unwrap();
    }

    #[tokio::test]
    async fn invalid_jwt_returns_error() {
        let root = filesys::Dir::create_temp_dir("install-test").await.unwrap();
        let layout = Layout::new(root.clone());
        let settings = Settings::default();
        let mock = MockClient::default();

        let result = install::install(&mock, &layout, &settings, "not-a-valid-jwt", None).await;

        assert!(result.is_err(), "expected error for invalid JWT");

        assert_eq!(mock.call_count(mock::Call::ActivateDevice), 0);

        // temp dir should still be cleaned up on error
        let temp_dir = layout.temp_dir();
        assert!(!temp_dir.exists(), "temp dir not deleted after error");

        root.delete().await.unwrap();
    }
}
