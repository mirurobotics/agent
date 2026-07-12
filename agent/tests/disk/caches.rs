// internal crates
use miru_agent::disk::{Capacities, DiskErr, Layout, Storage};
use miru_agent::filesys::dirs;
use miru_agent::models::{self, device};

// external crates
use tokio::time::Duration;

// Outer wall-clock net around join handles in each test. Purely hang
// protection -- its value is NOT part of the verified behavior. It
// must absorb coverage-instrumented, loaded-machine runs, so keep it
// generous; on success it never elapses and costs nothing.
const HANG_GUARD: Duration = Duration::from_secs(60);

pub mod default_capacities {
    use super::*;

    #[test]
    fn default() {
        let actual = Capacities::default();
        let expected = Capacities {
            cfg_insts: 1000,
            cfg_inst_content: 1000,
            deployments: 100,
            releases: 1000,
            upload_rules: 1000,
            git_commits: 100,
        };
        assert_eq!(actual, expected);
    }
}

pub mod init {
    use super::*;

    #[tokio::test]
    async fn init_and_shutdown() {
        let dir = dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());
        let capacities = Capacities::default();
        let (storage, _) = Storage::init(&layout, capacities, "test_device".to_string())
            .await
            .unwrap();

        // shutdown storage
        storage.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn shutdown_while_online() {
        let dir = dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());
        let capacities = Capacities::default();
        let (storage, _) = Storage::init(&layout, capacities, "test_device".to_string())
            .await
            .unwrap();

        // set device to online before shutdown
        storage
            .device
            .patch(device::Updates {
                status: Some(models::DeviceStatus::Online),
                ..device::Updates::empty()
            })
            .await
            .unwrap();

        // shutdown should transition device back to offline
        storage.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn shutdown_twice_returns_error() {
        let dir = dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());
        let capacities = Capacities::default();
        let (storage, _) = Storage::init(&layout, capacities, "test_device".to_string())
            .await
            .unwrap();

        // first shutdown succeeds
        storage.shutdown().await.unwrap();

        // second shutdown fails because the device actor is already stopped
        storage.shutdown().await.unwrap_err();
    }

    #[tokio::test]
    async fn shutdown_with_pre_closed_substore() {
        let dir = dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());
        let capacities = Capacities::default();
        let (storage, _) = Storage::init(&layout, capacities, "test_device".to_string())
            .await
            .unwrap();

        // pre-close the cfg_insts meta store
        storage.cfg_insts.meta.shutdown().await.unwrap();

        // shutdown fails when it reaches the already-closed cfg_insts.meta
        storage.shutdown().await.unwrap_err();
    }

    #[tokio::test]
    async fn shutdown_with_pre_closed_releases() {
        let dir = dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());
        let capacities = Capacities::default();
        let (storage, _) = Storage::init(&layout, capacities, "test_device".to_string())
            .await
            .unwrap();

        // pre-close the releases store
        storage.releases.shutdown().await.unwrap();

        // shutdown fails when it reaches the already-closed releases store
        storage.shutdown().await.unwrap_err();
    }

    #[tokio::test]
    async fn shutdown_with_pre_closed_upload_rules() {
        let dir = dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());
        let capacities = Capacities::default();
        let (storage, _) = Storage::init(&layout, capacities, "test_device".to_string())
            .await
            .unwrap();

        // pre-close the upload_rules store
        storage.upload_rules.shutdown().await.unwrap();

        // shutdown fails when it reaches the already-closed upload_rules store
        storage.shutdown().await.unwrap_err();
    }
}

pub mod shutdown {
    use super::*;

    #[tokio::test]
    async fn all_workers_exit_after_success() {
        let dir = dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());
        let (storage, storage_handle) =
            Storage::init(&layout, Capacities::default(), "test_device".to_string())
                .await
                .unwrap();

        storage.shutdown().await.unwrap();

        // all seven store workers exit, so the join handle completes
        tokio::time::timeout(HANG_GUARD, storage_handle)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn attempts_all_stores_after_early_failure() {
        let dir = dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());
        let (storage, storage_handle) =
            Storage::init(&layout, Capacities::default(), "test_device".to_string())
                .await
                .unwrap();

        // pre-close an early store so the shutdown sequence hits a dead worker
        storage.cfg_insts.meta.shutdown().await.unwrap();

        // shutdown reports the failure...
        let err = storage.shutdown().await.unwrap_err();
        assert!(matches!(err, DiskErr::CacheErr(_)));

        // ...but the later stores were still shut down, so every worker
        // exits and the join handle completes (pre-fix: times out)
        tokio::time::timeout(HANG_GUARD, storage_handle)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn returns_first_error_with_multiple_failures() {
        let dir = dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());
        let (storage, storage_handle) =
            Storage::init(&layout, Capacities::default(), "test_device".to_string())
                .await
                .unwrap();

        // pre-close the device store (fails first, at the device read
        // preamble) and the deployments store (fails later)
        storage.device.shutdown().await.unwrap();
        storage.deployments.shutdown().await.unwrap();

        // the FIRST error (the device read's FileSysErr) is returned,
        // not the later deployments CacheErr
        let err = storage.shutdown().await.unwrap_err();
        assert!(matches!(err, DiskErr::FileSysErr(_)));

        tokio::time::timeout(HANG_GUARD, storage_handle)
            .await
            .unwrap();
    }
}
