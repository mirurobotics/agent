// internal crates
use super::shared::{
    assert_temp_dir_cleaned, mock_failing_reprovision, mock_ok_reprovision, validate_storage, Env,
    StorageSnapshot,
};
use crate::mocks::http_client::{self as mock, MockClient};
use crate::test_utils::calibrations::{miru_calibration, report};
use backend_api::models::{CalibrationList, TokenResponse};
use miru_agent::calibrations::{latest, Filter, LatestStore, ReportMetadata};
use miru_agent::disk::Layout;
use miru_agent::filesys::{files, PathExt, WriteOptions};
use miru_agent::http::{errors::MockErr, HTTPErr};
use miru_agent::provisioning::{errors::*, reprovision};

/// Seed an upload queue, a queued report's file and a latest store entry for
/// `leg`.
async fn seed_calibrations(layout: &Layout) {
    let cals = layout.calibrations();
    let opts = WriteOptions::OVERWRITE_ATOMIC;
    files::write_string(&cals.queue(), r#"{"entries":[]}"#, opts)
        .await
        .unwrap();
    files::write_string(&cals.reports().file("queued"), "data", opts)
        .await
        .unwrap();
    let (store, worker) = LatestStore::spawn(64, cals.latest(), latest::DEFAULT_CAPACITY);
    store.put(report("leg").metadata, "old").await.unwrap();
    drop(store);
    worker.await.unwrap();
}

/// Every entry in the device's latest store, in key order.
async fn latest_entries(layout: &Layout) -> Vec<ReportMetadata> {
    let latest_layout = layout.calibrations().latest();
    let (store, _) = LatestStore::spawn(64, latest_layout, latest::DEFAULT_CAPACITY);
    store.list(&Filter::default()).await.unwrap()
}

fn assert_queue_deleted(layout: &Layout) {
    let cals = layout.calibrations();
    assert!(!cals.queue().exists(), "queue file still exists");
    assert!(!cals.reports().exists(), "reports dir still exists");
}

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
        assert_temp_dir_cleaned(&env.layout).await;

        env.cleanup().await;
    }

    #[tokio::test]
    async fn rotates_keypair() {
        let env = Env::new("reprovision-test").await;
        env.seed_provision("initial").await;
        let priv_before = files::read_string(&env.layout.auth().private_key())
            .await
            .unwrap();

        let mock = mock_ok_reprovision("after-reprovision");
        reprovision::reprovision(&mock, &env.layout, &env.settings, &env.token)
            .await
            .unwrap();

        let priv_after = files::read_string(&env.layout.auth().private_key())
            .await
            .unwrap();
        assert_ne!(
            priv_before, priv_after,
            "expected fresh keypair after reprovision"
        );

        env.cleanup().await;
    }

    #[tokio::test]
    async fn replaces_the_calibrations_with_mirus_latest() {
        let env = Env::new("reprovision-test").await;
        env.seed_provision("initial").await;
        seed_calibrations(&env.layout).await;
        let mock = mock_ok_reprovision("after-reprovision");
        mock.set_list_latest_calibrations(|| {
            Ok(CalibrationList {
                data: vec![miru_calibration("arm")],
                ..CalibrationList::default()
            })
        });

        reprovision::reprovision(&mock, &env.layout, &env.settings, &env.token)
            .await
            .unwrap();

        let expected = vec![report("arm").metadata];
        assert_eq!(latest_entries(&env.layout).await, expected);
        assert_queue_deleted(&env.layout);
        assert_temp_dir_cleaned(&env.layout).await;

        env.cleanup().await;
    }

    #[tokio::test]
    async fn a_failed_pull_wipes_the_calibrations_and_returns_calibrations_err() {
        let env = Env::new("reprovision-test").await;
        env.seed_provision("initial").await;
        seed_calibrations(&env.layout).await;
        let mock = mock_ok_reprovision("after-reprovision");
        mock.set_list_latest_calibrations(|| {
            Err(HTTPErr::MockErr(MockErr {
                is_network_conn_err: false,
            }))
        });

        let result = reprovision::reprovision(&mock, &env.layout, &env.settings, &env.token).await;

        assert!(matches!(result, Err(ProvisionErr::CalibrationsErr(_))));
        assert!(latest_entries(&env.layout).await.is_empty());
        assert_queue_deleted(&env.layout);
        validate_storage(&env.layout, "after-reprovision").await;

        env.cleanup().await;
    }

    #[tokio::test]
    async fn a_failed_token_wipes_the_calibrations() {
        let env = Env::new("reprovision-test").await;
        env.seed_provision("initial").await;
        seed_calibrations(&env.layout).await;
        // the default token response has an unparseable expiry
        let mock = MockClient {
            issue_device_token_fn: Box::new(|| Ok(TokenResponse::default())),
            ..mock_ok_reprovision("after-reprovision")
        };

        let result = reprovision::reprovision(&mock, &env.layout, &env.settings, &env.token).await;

        assert!(matches!(result, Err(ProvisionErr::AuthnErr(_))));
        assert!(latest_entries(&env.layout).await.is_empty());
        assert_queue_deleted(&env.layout);
        assert_temp_dir_cleaned(&env.layout).await;

        env.cleanup().await;
    }
}
