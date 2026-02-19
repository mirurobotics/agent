// internal crates
use miru_agent::filesys::dir::Dir;
use miru_agent::storage::caches::{CacheCapacities, Caches};
use miru_agent::storage::layout::StorageLayout;

pub mod default_capacities {
    use super::*;

    #[test]
    fn default() {
        let capacities = CacheCapacities::default();
        assert_eq!(capacities.cfg_inst, 100);
        assert_eq!(capacities.cfg_inst_content, 100);
        assert_eq!(capacities.deployment, 100);
    }
}

pub mod init {
    use super::*;

    #[tokio::test]
    async fn init_and_shutdown() {
        let dir = Dir::create_temp_dir("testing").await.unwrap();
        let layout = StorageLayout::new(dir);
        let capacities = CacheCapacities::default();
        let (caches, _) = Caches::init(&layout, capacities).await.unwrap();

        // shutdown the caches
        caches.shutdown().await.unwrap();
    }
}
