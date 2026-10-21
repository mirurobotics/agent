// standard crates
use std::path::PathBuf;

// internal crates
use crate::concurrent_cache_tests;
use crate::test_utils::filesys::dirs as test_dirs;
use miru_agent::cache::DirCache;
use miru_agent::filesys::{files, Overwrite, PathExt, WriteOptions};

// external crates
use tokio::task::JoinHandle;
#[allow(unused_imports)]
use tracing::{debug, error, info, trace, warn};

type TestCache = DirCache<String, String>;

async fn spawn_cache_with_capacity(
    capacity: usize,
) -> (test_dirs::TempDir, TestCache, JoinHandle<()>) {
    let tmp = test_dirs::temp("testing").unwrap();
    let dir = tmp.subdir(PathBuf::from("cache"));
    let (cache, handle) = TestCache::spawn(32, dir, capacity).await.unwrap();
    (tmp, cache, handle)
}

async fn spawn_cache() -> (test_dirs::TempDir, TestCache, JoinHandle<()>) {
    spawn_cache_with_capacity(1000).await
}

concurrent_cache_tests!(spawn_cache, spawn_cache_with_capacity);

#[tokio::test]
async fn spawn() {
    let tmp = test_dirs::temp("testing").unwrap();
    let dir = tmp.subdir(PathBuf::from("cache"));
    let _ = TestCache::spawn(32, dir.clone(), 1000).await.unwrap();
    // the directory should not exist yet
    assert!(dir.exists());

    // spawn again should not fail
    let _ = TestCache::spawn(32, dir.clone(), 1000).await.unwrap();
}

#[tokio::test]
async fn prune_invalid_entries() {
    let tmp = test_dirs::temp("testing").unwrap();
    let dir = tmp.subdir(PathBuf::from("cache"));
    let (cache, _) = TestCache::spawn(32, dir.clone(), 10).await.unwrap();

    // write invalid json files to files in the cache directory
    let invalid_json_file = dir.file("invalid.json");
    files::write_string(
        &invalid_json_file,
        "invalid json",
        WriteOptions::OVERWRITE_NONATOMIC,
    )
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
