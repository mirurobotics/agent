// internal crates
use miru_agent::filesys;
use miru_agent::storage::{Capacities, Layout, Storage};

pub mod default_capacities {
    use super::*;

    #[test]
    fn default() {
        let capacities = Capacities::default();
        assert_eq!(capacities.cfg_insts, 1000);
        assert_eq!(capacities.cfg_inst_content, 1000);
        assert_eq!(capacities.deployments, 100);
        assert_eq!(capacities.releases, 1000);
        assert_eq!(capacities.git_commits, 100);
    }
}

pub mod init {
    use super::*;

    #[tokio::test]
    async fn init_and_shutdown() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);
        let capacities = Capacities::default();
        let (storage, _) = Storage::init(&layout, capacities, "test_device".to_string())
            .await
            .unwrap();

        // shutdown storage
        storage.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn shutdown_while_online() {
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);
        let capacities = Capacities::default();
        let (storage, _) = Storage::init(&layout, capacities, "test_device".to_string())
            .await
            .unwrap();

        // set device to online before shutdown
        use miru_agent::models::{self, device};
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
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);
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
        let dir = filesys::Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);
        let capacities = Capacities::default();
        let (storage, _) = Storage::init(&layout, capacities, "test_device".to_string())
            .await
            .unwrap();

        // pre-close the cfg_insts meta store
        storage.cfg_insts.meta.shutdown().await.unwrap();

        // shutdown fails when it reaches the already-closed cfg_insts.meta
        storage.shutdown().await.unwrap_err();
    }
}
