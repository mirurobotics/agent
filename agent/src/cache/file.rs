// internal crates
use crate::cache::{
    concurrent::{Cache, Command, SendCacheKey, SendCacheValue, Worker},
    errors::CacheErr,
    file_impl::FileCacheImpl,
};
use crate::filesys::file::File;

// external crates
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub type FileCache<K, V> = Cache<FileCacheImpl<K, V>, K, V>;

impl<K, V> FileCache<K, V>
where
    K: SendCacheKey,
    V: SendCacheValue,
{
    pub async fn spawn(
        buffer_size: usize,
        file: File,
        capacity: usize,
    ) -> Result<(Self, JoinHandle<()>), CacheErr> {
        let (sender, receiver) = mpsc::channel::<Command<K, V>>(buffer_size);
        let worker = Worker {
            cache: FileCacheImpl::new(file, capacity).await?,
            receiver,
        };
        let worker_handle = tokio::spawn(worker.run());
        Ok((Self::new(sender), worker_handle))
    }
}
