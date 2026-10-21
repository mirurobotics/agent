// internal crates
use crate::cache::{
    concurrent::{Cache, Command, SendCacheKey, SendCacheValue, Worker},
    dir_impl::DirCacheImpl,
    errors::CacheErr,
};
use crate::filesys::dir::Dir;

// external crates
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub type DirCache<K, V> = Cache<DirCacheImpl<K, V>, K, V>;

impl<K, V> DirCache<K, V>
where
    K: SendCacheKey,
    V: SendCacheValue,
{
    pub async fn spawn(
        buffer_size: usize,
        dir: Dir,
        capacity: usize,
    ) -> Result<(Self, JoinHandle<()>), CacheErr> {
        let (sender, receiver) = mpsc::channel::<Command<K, V>>(buffer_size);
        let worker = Worker {
            cache: DirCacheImpl::new(dir, capacity).await?,
            receiver,
        };
        let worker_handle = tokio::spawn(worker.run());
        Ok((Self::new(sender), worker_handle))
    }
}
