// internal crates
use super::shared::{
    mock_failing_reprovision, mock_ok_reprovision, validate_storage, Env, StorageSnapshot,
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
