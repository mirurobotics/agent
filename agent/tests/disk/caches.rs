// internal crates
use crate::test_utils::{calibrations::report, filesys::dirs as test_dirs};
use miru_agent::calibrations::LatestStore;
use miru_agent::disk::{Capacities, DiskErr, Layout, Storage};
use miru_agent::filesys::{dirs, files, Overwrite, PathExt, WriteOptions};
use miru_agent::models::{self, device};

// external crates
use tokio::time::Duration;
use uuid::Uuid;

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
            file_rules: 1000,
            git_commits: 100,
            latest_cals: 1000,
        };
        assert_eq!(actual, expected);
    }
}

pub mod init {
    use super::*;

    #[tokio::test]
    async fn init_and_shutdown() {
        let dir = test_dirs::temp("testing").unwrap();
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
        let dir = test_dirs::temp("testing").unwrap();
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
    async fn init_fails_when_device_path_is_a_directory() {
        let dir = dirs::create_temp("testing").await.unwrap();
        let layout = Layout::new(dir);

        // occupy the device file path with a directory so its store fails to spawn
        dirs::create(&layout.root().subdir("device.json"))
            .await
            .unwrap();

        let result = Storage::init(&layout, Capacities::default(), "test_device".to_string()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn init_fails_when_cfg_inst_content_path_is_a_file() {
        let dir = dirs::create_temp("testing").await.unwrap();
        let layout = Layout::new(dir);

        // occupy the config instance content directory path with a file so its
        // store fails to spawn after device storage has initialized
        let contents_file = layout
            .resources()
            .subdir("config_instances")
            .file("contents");
        files::write_string(&contents_file, "not a directory", WriteOptions::default())
            .await
            .unwrap();

        let result = Storage::init(&layout, Capacities::default(), "test_device".to_string()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn init_keeps_the_latest_entries_and_deletes_orphaned_contents() {
        let dir = test_dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());
        let capacities = Capacities::default();
        let device_id = "test_device".to_string();
        let (storage, _) = Storage::init(&layout, capacities, device_id.clone())
            .await
            .unwrap();
        let named = report("a");
        let latest_cals = LatestStore::new(storage.latest_cals.clone());
        latest_cals
            .put(named.metadata.clone(), &named.data)
            .await
            .unwrap();
        let content = &storage.latest_cals.content;
        let orphan = Uuid::new_v4().to_string();
        content
            .write(orphan, named.data, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();
        storage.shutdown().await.unwrap();

        let (storage, _) = Storage::init(&layout, capacities, device_id).await.unwrap();

        let entries = storage.latest_cals.index.values().await.unwrap();
        assert_eq!(entries, std::slice::from_ref(&named.metadata));
        let contents = storage.latest_cals.content.entry_map().await.unwrap();
        let kept: Vec<String> = contents.into_keys().collect();
        assert_eq!(kept, [named.metadata.report_id.to_string()]);
        storage.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn init_keeps_the_latest_contents_when_the_index_cannot_be_read() {
        let dir = dirs::create_temp("testing").await.unwrap();
        let layout = Layout::new(dir);
        let latest_cals = layout.latest_cals();
        let index = latest_cals.root.subdir("index.json");
        dirs::create(&index).await.unwrap();
        dirs::create(&latest_cals.contents()).await.unwrap();
        let name = format!("{}.json", Uuid::new_v4());
        let content = latest_cals.contents().file(&name);
        files::write_string(&content, "x", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let result = Storage::init(&layout, Capacities::default(), "test_device".to_string()).await;

        // with no index to check them against, no content is deleted
        assert!(content.exists());
        result.unwrap().0.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn init_keeps_the_latest_contents_when_the_index_is_corrupt() {
        let dir = test_dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());
        let latest_cals = layout.latest_cals();
        dirs::create(&latest_cals.contents()).await.unwrap();
        files::write_string(&latest_cals.index(), "{", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();
        let name = format!("{}.json", Uuid::new_v4());
        let content = latest_cals.contents().file(&name);
        files::write_string(&content, "x", WriteOptions::OVERWRITE_ATOMIC)
            .await
            .unwrap();

        let result = Storage::init(&layout, Capacities::default(), "test_device".to_string()).await;

        assert!(content.exists());
        result.unwrap().0.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn init_fails_when_deployments_path_is_a_directory() {
        let dir = dirs::create_temp("testing").await.unwrap();
        let layout = Layout::new(dir);

        // occupy the deployments file path with a directory so its store fails
        // to spawn after the device and config instance stores have initialized
        dirs::create(&layout.resources().subdir("deployments.json"))
            .await
            .unwrap();

        let result = Storage::init(&layout, Capacities::default(), "test_device".to_string()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn shutdown_twice_returns_error() {
        let dir = test_dirs::temp("testing").unwrap();
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
        let dir = test_dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());
        let capacities = Capacities::default();
        let (storage, _) = Storage::init(&layout, capacities, "test_device".to_string())
            .await
            .unwrap();

        // pre-close the cfg_insts index store
        storage.cfg_insts.index.shutdown().await.unwrap();

        // shutdown fails when it reaches the already-closed cfg_insts.index
        storage.shutdown().await.unwrap_err();
    }

    #[tokio::test]
    async fn shutdown_with_pre_closed_releases() {
        let dir = test_dirs::temp("testing").unwrap();
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
    async fn shutdown_with_pre_closed_file_rules() {
        let dir = test_dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());
        let capacities = Capacities::default();
        let (storage, _) = Storage::init(&layout, capacities, "test_device".to_string())
            .await
            .unwrap();

        // pre-close the file_rules store
        storage.file_rules.shutdown().await.unwrap();

        // shutdown fails when it reaches the already-closed file_rules store
        storage.shutdown().await.unwrap_err();
    }
}

pub mod shutdown {
    use super::*;

    #[tokio::test]
    async fn all_workers_exit_after_success() {
        let dir = test_dirs::temp("testing").unwrap();
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
        let dir = test_dirs::temp("testing").unwrap();
        let layout = Layout::new(dir.to_dir());
        let (storage, storage_handle) =
            Storage::init(&layout, Capacities::default(), "test_device".to_string())
                .await
                .unwrap();

        // pre-close an early store so the shutdown sequence hits a dead worker
        storage.cfg_insts.index.shutdown().await.unwrap();

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
        let dir = test_dirs::temp("testing").unwrap();
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
