// internal crates
use crate::concurrent_cache_tests;
use crate::test_utils::filesys::dirs as test_dirs;
use miru_agent::cache::FileCache;
use miru_agent::filesys::PathExt;

// external crates
use tokio::task::JoinHandle;
#[allow(unused_imports)]
use tracing::{debug, error, info, trace, warn};

type TestCache = FileCache<String, String>;

async fn spawn_cache_with_capacity(
    capacity: usize,
) -> (test_dirs::TempDir, TestCache, JoinHandle<()>) {
    let tmp = test_dirs::temp("testing").unwrap();
    let file = tmp.file("cache.json");
    let (cache, handle) = TestCache::spawn(32, file, capacity).await.unwrap();
    (tmp, cache, handle)
}

async fn spawn_cache() -> (test_dirs::TempDir, TestCache, JoinHandle<()>) {
    spawn_cache_with_capacity(1000).await
}

pub mod spawn {
    use super::*;

    #[tokio::test]
    async fn spawn() {
        let tmp = test_dirs::temp("testing").unwrap();
        let file = tmp.file("cache.json");
        TestCache::spawn(32, file.clone(), 1000).await.unwrap();
        assert!(file.exists());

        // spawn again should not fail
        TestCache::spawn(32, file.clone(), 1000).await.unwrap();
    }
}

concurrent_cache_tests!(spawn_cache, spawn_cache_with_capacity);
