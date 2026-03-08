// internal crates
use crate::concurrent_cache_tests;
use crate::single_thread_cache_tests;
use miru_agent::cache::{FileCache, SingleThreadFileCache};
use miru_agent::filesys::{self, PathExt};

// external crates
use tokio::task::JoinHandle;
#[allow(unused_imports)]
use tracing::{debug, error, info, trace, warn};

pub mod concurrent {
    use super::*;

    type TestCache = FileCache<String, String>;

    async fn spawn_cache_with_capacity(capacity: usize) -> (TestCache, JoinHandle<()>) {
        let file = filesys::Dir::create_temp_dir("testing")
            .await
            .unwrap()
            .file("cache.json");
        TestCache::spawn(32, file.clone(), capacity).await.unwrap()
    }

    async fn spawn_cache() -> (TestCache, JoinHandle<()>) {
        spawn_cache_with_capacity(1000).await
    }

    pub mod spawn {
        use super::*;

        #[tokio::test]
        async fn spawn() {
            let file = filesys::Dir::create_temp_dir("testing")
                .await
                .unwrap()
                .file("cache.json");
            TestCache::spawn(32, file.clone(), 1000).await.unwrap();
            assert!(file.exists());

            // spawn again should not fail
            TestCache::spawn(32, file.clone(), 1000).await.unwrap();
        }
    }

    concurrent_cache_tests!(spawn_cache, spawn_cache_with_capacity);
}

pub mod single_thread {
    use super::*;

    type TestCache = SingleThreadFileCache<String, String>;

    async fn new_cache_with_capacity(capacity: usize) -> TestCache {
        let file = filesys::Dir::create_temp_dir("testing")
            .await
            .unwrap()
            .file("cache.json");
        TestCache::new(file.clone(), capacity).await.unwrap()
    }

    async fn new_cache() -> TestCache {
        new_cache_with_capacity(1000).await
    }

    pub mod new {
        use super::*;

        #[tokio::test]
        async fn new() {
            let file = filesys::Dir::create_temp_dir("testing")
                .await
                .unwrap()
                .file("cache.json");
            TestCache::new(file.clone(), 1000).await.unwrap();
            assert!(file.exists());

            // create again should not fail
            TestCache::new(file.clone(), 1000).await.unwrap();
        }
    }

    single_thread_cache_tests!(new_cache, new_cache_with_capacity);
}
