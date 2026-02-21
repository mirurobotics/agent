// internal crates
use miru_agent::filesys::dir::Dir;
use miru_agent::storage::{Capacities, Layout, Storage};

pub mod default_capacities {
    use super::*;

    #[test]
    fn default() {
        let capacities = Capacities::default();
        assert_eq!(capacities.cfg_insts, 100);
        assert_eq!(capacities.cfg_inst_content, 100);
        assert_eq!(capacities.deployments, 100);
    }
}

pub mod init {
    use super::*;

    #[tokio::test]
    async fn init_and_shutdown() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let layout = Layout::new(dir);
        let capacities = Capacities::default();
        let (storage, _) = Storage::init(&layout, capacities, "test_device".to_string())
            .await
            .unwrap();

        // shutdown storage
        storage.shutdown().await.unwrap();
    }
}
