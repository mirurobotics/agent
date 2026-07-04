// standard crates
use std::collections::HashMap;
use std::fmt::Debug;

// internal crates
use crate::cache::{
    concurrent::{Command, ConcurrentCache, ConcurrentCacheKey, ConcurrentCacheValue, Worker},
    entry::CacheEntry,
    errors::{CacheErr, CannotOverwriteCacheElement},
    single_thread::{CacheKey, CacheValue, SingleThreadCache},
};
use crate::filesys::{
    dir::Dir, dirs, file, file::File, files, path::PathExt, Atomic, Overwrite, WriteOptions,
};
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
        dirs::create_if_absent(&dir).await?;

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

        let entry = files::read_json::<CacheEntry<K, V>>(&entry_file).await?;

        Ok(Some(entry))
    }

    async fn write_entry_impl(
        &mut self,
        entry: &CacheEntry<K, V>,
        overwrite: Overwrite,
    ) -> Result<(), CacheErr> {
        let entry_file = self.cache_entry_file(&entry.key);
        if overwrite == Overwrite::Deny && entry_file.exists() {
            return Err(CacheErr::CannotOverwriteCacheElement(
                CannotOverwriteCacheElement {
                    key: entry.key.to_string(),
                    trace: trace!(),
                },
            ));
        }

        let opts = WriteOptions {
            overwrite,
            atomic: Atomic::Yes,
        };
        files::write_json(&entry_file, &entry, opts).await?;
        Ok(())
    }

    async fn delete_entry_impl(&mut self, key: &K) -> Result<(), CacheErr> {
        let entry_file = self.cache_entry_file(key);
        files::delete(&entry_file).await?;
        Ok(())
    }

    async fn size(&self) -> Result<usize, CacheErr> {
        if !self.dir.exists() {
            return Ok(0);
        }
        let files = dirs::files(&self.dir).await?;
        Ok(files.len())
    }

    async fn capacity(&self) -> Result<usize, CacheErr> {
        Ok(self.capacity)
    }

    async fn prune_invalid_entries(&self) -> Result<(), CacheErr> {
        let files = dirs::files(&self.dir).await?;
        let futures = files.into_iter().map(|file| async move {
            match files::read_json::<CacheEntry<K, V>>(&file).await {
                Ok(_) => Ok(()),
                Err(_) => files::delete(&file).await.map_err(CacheErr::from),
            }
        });
        try_join_all(futures).await?;
        Ok(())
    }

    async fn entries(&self) -> Result<Vec<CacheEntry<K, V>>, CacheErr> {
        let files = dirs::files(&self.dir).await?;
        let futures = files.into_iter().map(|file| async move {
            let result: Result<Option<CacheEntry<K, V>>, CacheErr> =
                match files::read_json::<CacheEntry<K, V>>(&file).await {
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
        let (sender, receiver) = mpsc::channel::<Command<K, V>>(buffer_size);
        let worker = Worker {
            cache: SingleThreadDirCache::new(dir, capacity).await?,
            receiver,
        };
        let worker_handle = tokio::spawn(worker.run());
        Ok((Self::new(sender), worker_handle))
    }
}
