// internal crates
use crate::concurrent_cache_tests;
use crate::single_thread_cache_tests;
use miru_agent::cache::{FileCache, SingleThreadFileCache};
use miru_agent::filesys::{dirs, PathExt};

// external crates
use tokio::task::JoinHandle;
#[allow(unused_imports)]
use tracing::{debug, error, info, trace, warn};

pub mod concurrent {
    use super::*;

    type TestCache = FileCache<String, String>;

    async fn spawn_cache_with_capacity(
        capacity: usize,
    ) -> (dirs::TempDir, TestCache, JoinHandle<()>) {
        let tmp = dirs::temp("testing").unwrap();
        let file = tmp.file("cache.json");
        let (cache, handle) = TestCache::spawn(32, file, capacity).await.unwrap();
        (tmp, cache, handle)
    }

    async fn spawn_cache() -> (dirs::TempDir, TestCache, JoinHandle<()>) {
        spawn_cache_with_capacity(1000).await
    }

    pub mod spawn {
        use super::*;

        #[tokio::test]
        async fn spawn() {
            let tmp = dirs::temp("testing").unwrap();
            let file = tmp.file("cache.json");
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

    async fn new_cache_with_capacity(capacity: usize) -> (dirs::TempDir, TestCache) {
        let tmp = dirs::temp("testing").unwrap();
        let file = tmp.file("cache.json");
        let cache = TestCache::new(file, capacity).await.unwrap();
        (tmp, cache)
    }

    async fn new_cache() -> (dirs::TempDir, TestCache) {
        new_cache_with_capacity(1000).await
    }

    pub mod new {
        use super::*;

        #[tokio::test]
        async fn new() {
            let tmp = dirs::temp("testing").unwrap();
            let file = tmp.file("cache.json");
            TestCache::new(file.clone(), 1000).await.unwrap();
            assert!(file.exists());

            // create again should not fail
            TestCache::new(file.clone(), 1000).await.unwrap();
        }
    }

    single_thread_cache_tests!(new_cache, new_cache_with_capacity);
}
