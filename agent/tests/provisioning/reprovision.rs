// internal crates
use super::shared::{
    assert_temp_dir_cleaned, mock_failing_reprovision, mock_ok_reprovision, validate_storage, Env,
    StorageSnapshot,
};
use crate::mocks::http_client as mock;
use crate::test_utils::calibrations::report;
use miru_agent::calibrations::{latest, Filter, LatestStore, ReportMetadata};
use miru_agent::disk::Layout;
use miru_agent::filesys::{files, File, PathExt, WriteOptions};
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
        seed_calibrations(&env.layout).await;
        let snapshot = StorageSnapshot::capture(&env.layout).await;

        let mock = mock_failing_reprovision();
        let result = reprovision::reprovision(&mock, &env.layout, &env.settings, &env.token).await;

        assert!(matches!(result, Err(ProvisionErr::HTTPErr(_))));
        snapshot.assert_unchanged(&env.layout).await;
        let cals = env.layout.calibrations();
        assert!(cals.queue().exists(), "queue file was deleted");
        assert!(cals.reports().exists(), "reports dir was deleted");
        let expected = vec![report("leg").metadata];
        assert_eq!(latest_entries(&env.layout).await, expected);
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
    async fn deletes_the_old_devices_calibrations_without_pulling() {
        let env = Env::new("reprovision-test").await;
        env.seed_provision("initial").await;
        seed_calibrations(&env.layout).await;
        let mock = mock_ok_reprovision("after-reprovision");

        reprovision::reprovision(&mock, &env.layout, &env.settings, &env.token)
            .await
            .unwrap();

        assert!(latest_entries(&env.layout).await.is_empty());
        assert_queue_deleted(&env.layout);
        assert_eq!(mock.call_count(mock::Call::ListLatestCalibrations), 0);
        assert_temp_dir_cleaned(&env.layout).await;

        env.cleanup().await;
    }

    #[tokio::test]
    async fn a_failed_calibrations_delete_fails_reprovision() {
        let env = Env::new("reprovision-test").await;
        env.seed_provision("initial").await;
        // a file where the calibrations dir belongs can't be deleted as a dir
        let cals_root = File::new(env.layout.calibrations().root.path().clone());
        files::write_string(&cals_root, "not a dir", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();
        let mock = mock_ok_reprovision("after-reprovision");

        let result = reprovision::reprovision(&mock, &env.layout, &env.settings, &env.token).await;

        assert!(
            matches!(result, Err(ProvisionErr::FileSysErr(_))),
            "{result:?}"
        );

        env.cleanup().await;
    }
}
