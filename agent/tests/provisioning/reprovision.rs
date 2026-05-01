// internal crates
use super::shared::{
    validate_storage, mock_failing_reprovision, mock_ok_reprovision, Env, StorageSnapshot,
};
use crate::mocks::http_client as mock;
use miru_agent::filesys::PathExt;
use miru_agent::provisioning::{errors::*, reprovision};

pub mod reprovision_fn {
    use super::*;

    #[tokio::test]
    async fn success() {
        let env = Env::new("reprovision-test").await;
        let mock = mock_ok_reprovision("after-reprovision");

        let device = reprovision::reprovision(&mock, &env.layout, &env.settings, &env.token)
            .await
            .unwrap();

        assert_eq!(device.name, "after-reprovision");
        assert_eq!(mock.call_count(mock::Call::ReprovisionDevice), 1);
        assert_eq!(mock.call_count(mock::Call::ProvisionDevice), 0);
        validate_storage(&env.layout, "after-reprovision").await;

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
            "missing public_key_pem in {body}"
        );
        assert!(
            body_value.get("agent_version").is_some(),
            "missing agent_version in {body}"
        );
        assert!(
            body_value.get("name").is_none(),
            "unexpected name field in {body}"
        );

        env.cleanup().await;
    }

    #[tokio::test]
    async fn http_error_preserves_existing_storage() {
        let env = Env::new("reprovision-test").await;
        env.seed_provision("initial").await;
        let snapshot = StorageSnapshot::capture(&env.layout).await;

        let mock = mock_failing_reprovision();
        let result = reprovision::reprovision(&mock, &env.layout, &env.settings, &env.token).await;

        assert!(matches!(result, Err(ProvisionErr::HTTPErr(_))));
        snapshot.assert_unchanged(&env.layout).await;
        assert!(!env.layout.temp_dir().exists(), "temp dir not cleaned");

        env.cleanup().await;
    }

    #[tokio::test]
    async fn rotates_keypair() {
        let env = Env::new("reprovision-test").await;
        env.seed_provision("initial").await;
        let priv_before = env.layout.auth().private_key().read_string().await.unwrap();

        let mock = mock_ok_reprovision("after-reprovision");
        reprovision::reprovision(&mock, &env.layout, &env.settings, &env.token)
            .await
            .unwrap();

        let priv_after = env.layout.auth().private_key().read_string().await.unwrap();
        assert_ne!(
            priv_before, priv_after,
            "expected fresh keypair after reprovision"
        );

        env.cleanup().await;
    }
}
