// internal crates
use miru_agent::authn::Token;
use miru_agent::crypt::base64;
use miru_agent::filesys::{self, WriteOptions};
use miru_agent::models::Device;
use miru_agent::storage::{assert_activated, resolve_device_id, Layout, StorageErr};

// external crates
use chrono::{Duration, Utc};

pub mod assert_activated {
    use super::*;

    #[tokio::test]
    async fn file_does_not_exist() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let device_file = dir.file("device.json");

        let result = assert_activated(&device_file).await.unwrap_err();
        assert!(matches!(result, StorageErr::FileSysErr { .. }));
    }

    #[tokio::test]
    async fn invalid_file_contents() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let device_file = dir.file("device.json");
        device_file
            .write_string("not a valid device", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let result = assert_activated(&device_file).await.unwrap_err();
        assert!(matches!(result, StorageErr::FileSysErr { .. }));
    }

    #[tokio::test]
    async fn device_not_activated() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let device_file = dir.file("device.json");
        let device = Device {
            activated: false,
            ..Default::default()
        };
        device_file
            .write_json(&device, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let result = assert_activated(&device_file).await.unwrap_err();
        assert!(matches!(result, StorageErr::DeviceNotActivatedErr { .. }));
    }

    #[tokio::test]
    async fn device_activated() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let device_file = dir.file("device.json");
        let device = Device {
            activated: true,
            ..Default::default()
        };
        device_file
            .write_json(&device, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        assert_activated(&device_file).await.unwrap();
    }
}

pub mod resolve_device_id {
    use super::*;

    fn jwt_with_sub(device_id: &str) -> String {
        let payload = serde_json::json!({
            "iss": "miru",
            "aud": "device",
            "exp": 1_721_517_034u64,
            "iat": 1_721_495_434u64,
            "sub": device_id,
        })
        .to_string();
        format!(
            "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.{}.UIqAz_V-ZuZLIHUXwLHw-A2CrXBQrpXnJAMlVfmMXYY",
            base64::encode_string_url_safe_no_pad(&payload)
        )
    }

    #[tokio::test]
    async fn returns_id_from_device_file_when_valid() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);

        let device = Device {
            id: "dvc_from_file".to_string(),
            ..Device::default()
        };
        layout
            .device()
            .write_json(&device, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let id = resolve_device_id(&layout).await.unwrap();
        assert_eq!(id, "dvc_from_file");
    }

    #[tokio::test]
    async fn falls_back_to_token_jwt_when_device_file_missing() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);

        // no device.json — write a token.json containing a JWT with the
        // device id encoded in the `sub` claim
        let auth = layout.auth();
        auth.root.create_if_absent().await.unwrap();
        let token = Token {
            token: jwt_with_sub("dvc_from_jwt"),
            expires_at: Utc::now() + Duration::minutes(5),
        };
        auth.token()
            .write_json(&token, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let id = resolve_device_id(&layout).await.unwrap();
        assert_eq!(id, "dvc_from_jwt");
    }

    #[tokio::test]
    async fn returns_resolve_err_when_no_sources_yield_id() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);

        // empty layout: no device.json, no token.json
        let err = resolve_device_id(&layout).await.unwrap_err();
        assert!(matches!(err, StorageErr::ResolveDeviceIDErr(_)));
    }
}
