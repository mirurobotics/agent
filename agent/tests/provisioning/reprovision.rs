// internal crates
use super::shared::{new_device, new_jwt, DEVICE_ID};
use crate::mocks::http_client::{self as mock, MockClient};
use miru_agent::filesys::{self, PathExt};
use miru_agent::http::HTTPErr;
use miru_agent::provisioning::{errors::*, provision, reprovision};
use miru_agent::storage::{Layout, Settings};

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

        let device = reprovision::reprovision(&mock, &layout, &settings, &token)
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
        let result = reprovision::reprovision(&mock_fail, &layout, &settings, &token).await;
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
        reprovision::reprovision(&mock_reprovision, &layout, &settings, &token)
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
