// standard crates
#[cfg(unix)]
use std::{fs::Permissions, os::unix::fs::PermissionsExt};

// internal crates
use crate::concurrent_cache_tests;
use crate::single_thread_cache_tests;
use crate::test_utils::filesys::dirs as test_dirs;
#[cfg(unix)]
use crate::test_utils::filesys::{assert_file_mode, files as test_files};
use miru_agent::cache::{FileCache, SingleThreadFileCache};
use miru_agent::filesys::PathExt;

// external crates
use tokio::task::JoinHandle;
#[allow(unused_imports)]
use tracing::{debug, error, info, trace, warn};

pub mod concurrent {
    use super::*;

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
}

pub mod single_thread {
    use super::*;

    type TestCache = SingleThreadFileCache<String, String>;

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

        #[cfg(unix)]
        #[tokio::test]
        async fn new_creates_file_0600() {
            let tmp = test_dirs::temp("testing").unwrap();
            let file = tmp.file("cache.json");
            TestCache::new(file.clone(), 1000).await.unwrap();
            assert_file_mode(&file, 0o600).await;
        }
    }

    // a legacy cache file keeps its mode until the next write replaces it
    #[cfg(unix)]
    #[tokio::test]
    async fn write_tightens_existing_file_to_0600() {
        // internal crates
        use miru_agent::cache::single_thread::SingleThreadCache;
        use miru_agent::filesys::{files, Overwrite};

        let tmp = test_dirs::temp("testing").unwrap();
        let file = tmp.file("cache.json");
        test_files::seed(&file, "{}").await;
        files::set_permissions(&file, Permissions::from_mode(0o644))
            .await
            .unwrap();

        let mut cache = TestCache::new(file.clone(), 1000).await.unwrap();
        assert_file_mode(&file, 0o644).await;

        cache
            .write("k".into(), "v".into(), |_, _| false, Overwrite::Allow)
            .await
            .unwrap();
        assert_file_mode(&file, 0o600).await;
    }

    single_thread_cache_tests!(new_cache, new_cache_with_capacity);
}
