// standard library
use std::path::PathBuf;

// internal crates
use crate::{concurrent_cache_tests, single_thread_cache_tests};
use miru_agent::cache::dir::{DirCache, SingleThreadDirCache};
use miru_agent::crud::prelude::*;
use miru_agent::filesys::{dir::Dir, path::PathExt, Overwrite, WriteOptions};

// external crates
use tokio::task::JoinHandle;
#[allow(unused_imports)]
use tracing::{debug, error, info, trace, warn};

pub mod concurrent {
    use super::*;

    type TestCache = DirCache<String, String>;

    async fn spawn_cache_with_capacity(capacity: usize) -> (TestCache, JoinHandle<()>) {
        let dir = Dir::create_temp_dir("testing")
            .await
            .unwrap()
            .subdir(PathBuf::from("cache"));
        TestCache::spawn(32, dir.clone(), capacity).await.unwrap()
    }

    async fn spawn_cache() -> (TestCache, JoinHandle<()>) {
        spawn_cache_with_capacity(1000).await
    }

    concurrent_cache_tests!(spawn_cache, spawn_cache_with_capacity);

    #[tokio::test]
    async fn spawn() {
        let dir = Dir::create_temp_dir("testing")
            .await
            .unwrap()
            .subdir(PathBuf::from("cache"));
        let _ = TestCache::spawn(32, dir.clone(), 1000).await.unwrap();
        // the directory should not exist yet
        assert!(dir.exists());

        // spawn again should not fail
        let _ = TestCache::spawn(32, dir.clone(), 1000).await.unwrap();
    }

    #[tokio::test]
    async fn prune_invalid_entries() {
        let dir = Dir::create_temp_dir("testing")
            .await
            .unwrap()
            .subdir(PathBuf::from("cache"));
        let (cache, _) = TestCache::spawn(32, dir.clone(), 10).await.unwrap();

        // write invalid json files to files in the cache directory
        let invalid_json_file = dir.file("invalid.json");
        invalid_json_file
            .write_string("invalid json", WriteOptions::OVERWRITE)
            .await
            .unwrap();

        // create 10 entries
        for i in 0..10 {
            let key = format!("key{i}");
            let value = format!("value{i}");
            cache
                .write(key, value, |_, _| true, Overwrite::Deny)
                .await
                .unwrap();
        }

        // prune the cache
        cache.prune().await.unwrap();

        // invalid json file should be deleted
        assert!(!invalid_json_file.exists());

        // the cache should still have all ten entries
        for i in 0..10 {
            let key = format!("key{i}");
            let value = cache.read(key).await.unwrap();
            assert_eq!(value, format!("value{i}"));
        }
    }
}

pub mod single_thread {
    use super::*;

    type TestCache = SingleThreadDirCache<String, String>;

    async fn new_cache_with_capacity(capacity: usize) -> TestCache {
        let dir = Dir::create_temp_dir("testing")
            .await
            .unwrap()
            .subdir(PathBuf::from("cache"));
        TestCache::new(dir.clone(), capacity).await.unwrap()
    }

    async fn new_cache() -> TestCache {
        new_cache_with_capacity(1000).await
    }

    #[tokio::test]
    async fn new() {
        let dir = Dir::create_temp_dir("testing")
            .await
            .unwrap()
            .subdir(PathBuf::from("cache"));
        let _ = TestCache::new(dir.clone(), 1000).await.unwrap();
        assert!(dir.exists());

        // new should not fail
        let _ = TestCache::new(dir.clone(), 1000).await.unwrap();
    }

    single_thread_cache_tests!(new_cache, new_cache_with_capacity);
}
