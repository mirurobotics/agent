// internal crates
use crate::cache_impl_tests;
use crate::test_utils::filesys::dirs as test_dirs;
use miru_agent::cache::FileCacheImpl;
use miru_agent::filesys::PathExt;

// external crates
#[allow(unused_imports)]
use tracing::{debug, error, info, trace, warn};

type TestCache = FileCacheImpl<String, String>;

async fn new_cache_with_capacity(capacity: usize) -> (test_dirs::TempDir, TestCache) {
    let tmp = test_dirs::temp("testing").unwrap();
    let file = tmp.file("cache.json");
    let cache = TestCache::new(file, capacity).await.unwrap();
    (tmp, cache)
}

async fn new_cache() -> (test_dirs::TempDir, TestCache) {
    new_cache_with_capacity(1000).await
}

pub mod new {
    use super::*;

    #[tokio::test]
    async fn new() {
        let tmp = test_dirs::temp("testing").unwrap();
        let file = tmp.file("cache.json");
        TestCache::new(file.clone(), 1000).await.unwrap();
        assert!(file.exists());

        // create again should not fail
        TestCache::new(file.clone(), 1000).await.unwrap();
    }
}

cache_impl_tests!(new_cache, new_cache_with_capacity);
