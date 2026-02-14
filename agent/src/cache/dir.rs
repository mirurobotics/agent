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
};
use crate::filesys::{dir::Dir, file, file::File, path::PathExt};
use crate::trace;

// external crates
use futures::future::try_join_all;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

#[derive(Debug)]
pub struct SingleThreadDirCache<K, V>
where
    K: CacheKey,
    V: CacheValue,
{
    dir: Dir,
    capacity: usize,
    _phantom: std::marker::PhantomData<K>,
    _phantom2: std::marker::PhantomData<V>,
}

impl<K, V> SingleThreadDirCache<K, V>
where
    K: CacheKey,
    V: CacheValue,
{
    pub async fn new(dir: Dir, capacity: usize) -> Result<Self, CacheErr> {
        dir.create_if_absent().await?;

        Ok(Self {
            dir,
            capacity,
            _phantom: std::marker::PhantomData,
            _phantom2: std::marker::PhantomData,
        })
    }

    fn cache_entry_file(&self, key: &K) -> File {
        let mut filename = format!("{}.json", key.to_string());
        filename = file::sanitize_filename(&filename);
        self.dir.file(&filename)
    }
}

impl<K, V> SingleThreadCache<K, V> for SingleThreadDirCache<K, V>
where
    K: CacheKey,
    V: CacheValue,
{
    async fn read_entry_impl(&self, key: &K) -> Result<Option<CacheEntry<K, V>>, CacheErr> {
        let entry_file = self.cache_entry_file(key);
        if !entry_file.exists() {
            return Ok(None);
        }

        let entry = entry_file.read_json::<CacheEntry<K, V>>().await?;

        Ok(Some(entry))
    }

    async fn write_entry_impl(
        &mut self,
        entry: &CacheEntry<K, V>,
        overwrite: bool,
    ) -> Result<(), CacheErr> {
        let atomic = true;
        let entry_file = self.cache_entry_file(&entry.key);
        if !overwrite && entry_file.exists() {
            return Err(CacheErr::CannotOverwriteCacheElement(
                CannotOverwriteCacheElement {
                    key: entry.key.to_string(),
                    trace: trace!(),
                },
            ));
        }

        entry_file.write_json(&entry, overwrite, atomic).await?;
        Ok(())
    }

    async fn delete_entry_impl(&mut self, key: &K) -> Result<(), CacheErr> {
        let entry_file = self.cache_entry_file(key);
        entry_file.delete().await?;
        Ok(())
    }

    async fn size(&self) -> Result<usize, CacheErr> {
        if !self.dir.exists() {
            return Ok(0);
        }
        let files = self.dir.files().await?;
        Ok(files.len())
    }

    async fn capacity(&self) -> Result<usize, CacheErr> {
        Ok(self.capacity)
    }

    async fn prune_invalid_entries(&self) -> Result<(), CacheErr> {
        let files = self.dir.files().await?;
        let futures = files.into_iter().map(|file| async move {
            match file.read_json::<CacheEntry<K, V>>().await {
                Ok(_) => Ok(()),
                Err(_) => file.delete().await.map_err(CacheErr::from),
            }
        });
        try_join_all(futures).await?;
        Ok(())
    }

    async fn entries(&self) -> Result<Vec<CacheEntry<K, V>>, CacheErr> {
        let files = self.dir.files().await?;
        let futures = files.into_iter().map(|file| async move {
            let result: Result<Option<CacheEntry<K, V>>, CacheErr> =
                match file.read_json::<CacheEntry<K, V>>().await {
                    Ok(entry) => Ok(Some(entry)),
                    Err(_) => Ok(None),
                };
            result
        });
        let entries = try_join_all(futures).await?;
        Ok(entries.into_iter().flatten().collect())
    }

    async fn values(&self) -> Result<Vec<V>, CacheErr> {
        let entries = self.entries().await?;
        Ok(entries.into_iter().map(|e| e.value).collect())
    }

    async fn entry_map(&self) -> Result<HashMap<K, CacheEntry<K, V>>, CacheErr> {
        let entries = self.entries().await?;
        Ok(entries.into_iter().map(|e| (e.key.clone(), e)).collect())
    }

    async fn value_map(&self) -> Result<HashMap<K, V>, CacheErr> {
        let entries = self.entries().await?;
        Ok(entries.into_iter().map(|e| (e.key, e.value)).collect())
    }
}

pub type DirCache<K, V> = ConcurrentCache<SingleThreadDirCache<K, V>, K, V>;

impl<K, V> DirCache<K, V>
where
    K: ConcurrentCacheKey,
    V: ConcurrentCacheValue,
{
    pub async fn spawn(
        buffer_size: usize,
        dir: Dir,
        capacity: usize,
    ) -> Result<(Self, JoinHandle<()>), CacheErr> {
        let (sender, receiver) = mpsc::channel::<WorkerCommand<K, V>>(buffer_size);
        let worker = Worker {
            cache: SingleThreadDirCache::new(dir, capacity).await?,
            receiver,
        };
        let worker_handle = tokio::spawn(worker.run());
        Ok((Self::new(sender), worker_handle))
    }
}
