// standard crates
use std::path::PathBuf;

// internal crates
use crate::cache_impl_tests;
use crate::test_utils::filesys::{dirs as test_dirs, files as test_files};
use miru_agent::cache::{CacheImpl, DirCacheImpl};
use miru_agent::filesys::{Overwrite, PathExt};

// external crates
#[allow(unused_imports)]
use tracing::{debug, error, info, trace, warn};

type TestCache = DirCacheImpl<String, String>;

async fn new_cache_with_capacity(capacity: usize) -> (test_dirs::TempDir, TestCache) {
    let tmp = test_dirs::temp("testing").unwrap();
    let dir = tmp.subdir(PathBuf::from("cache"));
    let cache = TestCache::new(dir, capacity).await.unwrap();
    (tmp, cache)
}

async fn new_cache() -> (test_dirs::TempDir, TestCache) {
    new_cache_with_capacity(1000).await
}

#[tokio::test]
async fn new() {
    let tmp = test_dirs::temp("testing").unwrap();
    let dir = tmp.subdir(PathBuf::from("cache"));
    let _ = TestCache::new(dir.clone(), 1000).await.unwrap();
    assert!(dir.exists());

    // new should not fail
    let _ = TestCache::new(dir.clone(), 1000).await.unwrap();
}

cache_impl_tests!(new_cache, new_cache_with_capacity);

#[tokio::test]
async fn prune_invalid_entries_reduces_below_capacity() {
    let tmp = test_dirs::temp("testing").unwrap();
    let dir = tmp.subdir(PathBuf::from("cache"));
    let mut cache = TestCache::new(dir.clone(), 5).await.unwrap();

    // write 3 valid entries
    for i in 0..3 {
        let key = format!("key{i}");
        let value = format!("value{i}");
        cache
            .write(key, value, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();
    }

    // inject 3 invalid (non-JSON) files to push size to 6 (> capacity 5)
    for i in 0..3 {
        let invalid_file = dir.file(&format!("invalid{i}.json"));
        test_files::seed(&invalid_file, "not valid json").await;
    }

    // size should now be 6
    assert_eq!(cache.size().await.unwrap(), 6);

    // prune should not panic or delete valid entries
    cache.prune().await.unwrap();

    // all 3 valid entries should still be readable
    for i in 0..3 {
        let key = format!("key{i}");
        let value = cache.read(&key).await.unwrap();
        assert_eq!(value, format!("value{i}"));
    }

    // total size should be 3 (invalid files removed, valid entries kept)
    assert_eq!(cache.size().await.unwrap(), 3);
}
