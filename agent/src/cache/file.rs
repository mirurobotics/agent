// standard library
use std::collections::HashMap;
use std::fmt::Debug;

// internal crates
use crate::cache::{
    concurrent::{
        ConcurrentCache, ConcurrentCacheKey, ConcurrentCacheValue, Worker, WorkerCommand,
    },
    entry::CacheEntry,
    errors::{CacheErr, CannotOverwriteCacheElement},
    single_thread::{CacheKey, CacheValue, SingleThreadCache},
    Overwrite,
};
use crate::filesys::{file::File, path::PathExt, WriteOptions};
use crate::trace;

// external crates
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

#[derive(Debug)]
pub struct SingleThreadFileCache<K, V>
where
    K: CacheKey,
    V: CacheValue,
{
    file: File,
    capacity: usize,
    _phantom: std::marker::PhantomData<K>,
    _phantom2: std::marker::PhantomData<V>,
}

impl<K, V> SingleThreadFileCache<K, V>
where
    K: CacheKey,
    V: CacheValue,
{
    pub async fn new(file: File, capacity: usize) -> Result<Self, CacheErr> {
        if !file.exists() {
            let empty_cache: HashMap<K, CacheEntry<K, V>> = HashMap::new();
            file.write_json(&empty_cache, WriteOptions::OVERWRITE_ATOMIC)
                .await?;
        }

        Ok(Self {
            file,
            capacity,
            _phantom: std::marker::PhantomData,
            _phantom2: std::marker::PhantomData,
        })
    }

    async fn read_cache(&self) -> Result<HashMap<K, CacheEntry<K, V>>, CacheErr> {
        self.file
            .read_json::<HashMap<K, CacheEntry<K, V>>>()
            .await
            .map_err(CacheErr::from)
    }

    async fn write_cache(&self, cache: &HashMap<K, CacheEntry<K, V>>) -> Result<(), CacheErr> {
        self.file
            .write_json(cache, WriteOptions::OVERWRITE_ATOMIC)
            .await
            .map_err(CacheErr::from)
    }
}

impl<K, V> SingleThreadCache<K, V> for SingleThreadFileCache<K, V>
where
    K: CacheKey,
    V: CacheValue,
{
    async fn read_entry_impl(&self, key: &K) -> Result<Option<CacheEntry<K, V>>, CacheErr> {
        let cache = self.read_cache().await?;
        Ok(cache.get(key).cloned())
    }

    async fn write_entry_impl(
        &mut self,
        entry: &CacheEntry<K, V>,
        overwrite: Overwrite,
    ) -> Result<(), CacheErr> {
        let mut cache = self.read_cache().await?;
        if overwrite == Overwrite::Deny && cache.contains_key(&entry.key) {
            return Err(CacheErr::CannotOverwriteCacheElement(
                CannotOverwriteCacheElement {
                    key: entry.key.to_string(),
                    trace: trace!(),
                },
            ));
        }
        cache.insert(entry.key.clone(), entry.clone());
        self.write_cache(&cache).await?;
        Ok(())
    }

    async fn delete_entry_impl(&mut self, key: &K) -> Result<(), CacheErr> {
        let mut cache = self.read_cache().await?;
        cache.remove(key);
        self.write_cache(&cache).await?;
        Ok(())
    }

    async fn size(&self) -> Result<usize, CacheErr> {
        let cache = self.read_cache().await?;
        Ok(cache.len())
    }

    async fn capacity(&self) -> Result<usize, CacheErr> {
        Ok(self.capacity)
    }

    async fn prune_invalid_entries(&self) -> Result<(), CacheErr> {
        Ok(())
    }

    async fn entries(&self) -> Result<Vec<CacheEntry<K, V>>, CacheErr> {
        let cache = self.read_cache().await?;
        Ok(cache.values().cloned().collect())
    }

    async fn values(&self) -> Result<Vec<V>, CacheErr> {
        let cache = self.read_cache().await?;
        Ok(cache.values().map(|v| v.value.clone()).collect())
    }

    async fn entry_map(&self) -> Result<HashMap<K, CacheEntry<K, V>>, CacheErr> {
        let cache = self.read_cache().await?;
        Ok(cache)
    }

    async fn value_map(&self) -> Result<HashMap<K, V>, CacheErr> {
        let cache = self.read_cache().await?;
        Ok(cache.into_iter().map(|(k, v)| (k, v.value)).collect())
    }
}

pub type FileCache<K, V> = ConcurrentCache<SingleThreadFileCache<K, V>, K, V>;

impl<K, V> FileCache<K, V>
where
    K: ConcurrentCacheKey,
    V: ConcurrentCacheValue,
{
    pub async fn spawn(
        buffer_size: usize,
        file: File,
        capacity: usize,
    ) -> Result<(Self, JoinHandle<()>), CacheErr> {
        let (sender, receiver) = mpsc::channel::<WorkerCommand<K, V>>(buffer_size);
        let worker = Worker {
            cache: SingleThreadFileCache::new(file, capacity).await?,
            receiver,
        };
        let worker_handle = tokio::spawn(worker.run());
        Ok((Self::new(sender), worker_handle))
    }
}
