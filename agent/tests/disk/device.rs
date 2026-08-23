// internal crates
use miru_agent::authn::Token;
use miru_agent::crypt::base64;
use miru_agent::disk::{
    activation_state, assert_activated, resolve_device_id, Activation, DiskErr, Layout,
};
use miru_agent::filesys::{dirs, files, WriteOptions};
use miru_agent::models::Device;

// external crates
use chrono::{Duration, Utc};

pub mod assert_activated {
    use super::*;

    async fn fresh_layout() -> (Layout, dirs::TempDir) {
        let dir = dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());
        dirs::create_if_absent(&layout.auth().root).await.unwrap();
        (layout, dir)
    }

    #[tokio::test]
    async fn returns_err_when_both_keys_missing() {
        let (layout, _tmp) = fresh_layout().await;

        let result = assert_activated(&layout).unwrap_err();
        assert!(matches!(result, DiskErr::DeviceNotActivatedErr(_)));
    }

    #[tokio::test]
    async fn returns_err_when_private_key_missing() {
        let (layout, _tmp) = fresh_layout().await;
        files::seed(&layout.auth().public_key(), "public").await;

        let result = assert_activated(&layout).unwrap_err();
        assert!(matches!(result, DiskErr::DeviceNotActivatedErr(_)));
    }

    #[tokio::test]
    async fn returns_err_when_public_key_missing() {
        let (layout, _tmp) = fresh_layout().await;
        files::seed(&layout.auth().private_key(), "private").await;

        let result = assert_activated(&layout).unwrap_err();
        assert!(matches!(result, DiskErr::DeviceNotActivatedErr(_)));
    }

    #[tokio::test]
    async fn returns_ok_when_both_keys_present() {
        let (layout, _tmp) = fresh_layout().await;
        let auth = layout.auth();
        files::seed(&auth.private_key(), "private").await;
        files::seed(&auth.public_key(), "public").await;

        assert_activated(&layout).unwrap();
    }
}

pub mod activation_state {
    use super::*;

    // standard crates
    use std::os::unix::fs::PermissionsExt;

    async fn fresh_layout() -> (Layout, dirs::TempDir) {
        let dir = dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());
        dirs::create_if_absent(&layout.auth().root).await.unwrap();
        (layout, dir)
    }

    #[tokio::test]
    async fn not_activated_when_both_keys_missing() {
        let (layout, _tmp) = fresh_layout().await;

        assert_eq!(Activation::NotActivated, activation_state(&layout).unwrap());
    }

    #[tokio::test]
    async fn not_activated_when_public_key_missing() {
        let (layout, _tmp) = fresh_layout().await;
        files::seed(&layout.auth().private_key(), "private").await;

        assert_eq!(Activation::NotActivated, activation_state(&layout).unwrap());
    }

    #[tokio::test]
    async fn not_activated_when_private_key_missing() {
        let (layout, _tmp) = fresh_layout().await;
        files::seed(&layout.auth().public_key(), "public").await;

        assert_eq!(Activation::NotActivated, activation_state(&layout).unwrap());
    }

    #[tokio::test]
    async fn activated_when_both_keys_present() {
        let (layout, _tmp) = fresh_layout().await;
        let auth = layout.auth();
        files::seed(&auth.private_key(), "private").await;
        files::seed(&auth.public_key(), "public").await;

        assert_eq!(Activation::Activated, activation_state(&layout).unwrap());
    }

    #[tokio::test]
    async fn errs_when_auth_dir_is_unreadable() {
        // root ignores permission bits, so the error cannot be provoked
        if nix::unistd::geteuid().is_root() {
            eprintln!("skipping: running as root, permission bits are bypassed");
            return;
        }

        let (layout, _tmp) = fresh_layout().await;
        let auth = layout.auth();
        files::seed(&auth.private_key(), "private").await;
        files::seed(&auth.public_key(), "public").await;

        dirs::set_permissions(&auth.root, std::fs::Permissions::from_mode(0o000))
            .await
            .unwrap();
        let result = activation_state(&layout);
        // restore before asserting so a failure cannot leak an unreadable dir
        dirs::set_permissions(&auth.root, std::fs::Permissions::from_mode(0o755))
            .await
            .unwrap();

        assert!(
            matches!(result, Err(DiskErr::FileSysErr(_))),
            "expected FileSysErr, got {result:?}"
        );
    }
}

pub mod resolve_device_id {
    use super::*;

    fn new_jwt(device_id: &str) -> String {
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
        let dir = dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());

        let device = Device {
            id: "dvc_from_file".to_string(),
            ..Device::default()
        };
        files::write_json(&layout.device(), &device, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let id = resolve_device_id(&layout).await.unwrap();
        assert_eq!(id, "dvc_from_file");
    }

    #[tokio::test]
    async fn falls_back_to_token_jwt_when_device_file_missing() {
        let dir = dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());

        // no device.json — write a token.json containing a JWT with the
        // device id encoded in the `sub` claim
        let auth = layout.auth();
        dirs::create_if_absent(&auth.root).await.unwrap();
        let token = Token {
            token: new_jwt("dvc_from_jwt"),
            expires_at: Utc::now() + Duration::minutes(5),
        };
        files::write_json(&auth.token(), &token, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let id = resolve_device_id(&layout).await.unwrap();
        assert_eq!(id, "dvc_from_jwt");
    }

    #[tokio::test]
    async fn returns_resolve_err_when_no_sources_yield_id() {
        let dir = dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());

        // empty layout: no device.json, no token.json
        let err = resolve_device_id(&layout).await.unwrap_err();
        assert!(matches!(err, DiskErr::ResolveDeviceIDErr(_)));
    }
}
