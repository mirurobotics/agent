// standard library
use std::collections::HashMap;
use std::fmt::Debug;

// internal crates
use crate::cache::{
    entry::CacheEntry,
    errors::{CacheErr, ReceiveActorMessageErr, SendActorMessageErr},
    single_thread::{CacheKey, CacheValue, SingleThreadCache},
    Overwrite,
};
use crate::crud::{errors::CrudErr, prelude::*};
use crate::trace;

// external crates
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::oneshot;
use tracing::{error, info};

pub trait ConcurrentCacheKey: CacheKey + Send + Sync + 'static {}

impl<K> ConcurrentCacheKey for K where K: CacheKey + Send + Sync + 'static {}

pub trait ConcurrentCacheValue: CacheValue + Send + Sync + 'static {}

impl<V> ConcurrentCacheValue for V where V: CacheValue + Send + Sync + 'static {}

// ============================== WORKER COMMANDS ================================== //
type QueryEntryFilter<K, V> = Box<dyn Fn(&CacheEntry<K, V>) -> bool + Send + Sync>;
type QueryValueFilter<V> = Box<dyn Fn(&V) -> bool + Send + Sync>;
type IsDirty<K, V> = Box<dyn Fn(Option<&CacheEntry<K, V>>, &V) -> bool + Send + Sync>;
type CacheEntryMap<K, V> = HashMap<K, CacheEntry<K, V>>;

pub enum WorkerCommand<K, V>
where
    K: Clone + Send + Sync + ToString + Serialize + DeserializeOwned,
    V: Clone + Send + Sync + Serialize + DeserializeOwned,
{
    Shutdown {
        respond_to: oneshot::Sender<Result<(), CacheErr>>,
    },
    ReadEntryOptional {
        key: K,
        respond_to: oneshot::Sender<Result<Option<CacheEntry<K, V>>, CacheErr>>,
    },
    ReadEntry {
        key: K,
        respond_to: oneshot::Sender<Result<CacheEntry<K, V>, CacheErr>>,
    },
    ReadOptional {
        key: K,
        respond_to: oneshot::Sender<Result<Option<V>, CacheErr>>,
    },
    Read {
        key: K,
        respond_to: oneshot::Sender<Result<V, CacheErr>>,
    },
    Write {
        key: K,
        value: V,
        is_dirty: IsDirty<K, V>,
        overwrite: Overwrite,
        respond_to: oneshot::Sender<Result<(), CacheErr>>,
    },
    Delete {
        key: K,
        respond_to: oneshot::Sender<Result<(), CacheErr>>,
    },
    Prune {
        respond_to: oneshot::Sender<Result<(), CacheErr>>,
    },
    Size {
        respond_to: oneshot::Sender<Result<usize, CacheErr>>,
    },
    Entries {
        respond_to: oneshot::Sender<Result<Vec<CacheEntry<K, V>>, CacheErr>>,
    },
    Values {
        respond_to: oneshot::Sender<Result<Vec<V>, CacheErr>>,
    },
    EntryMap {
        respond_to: oneshot::Sender<Result<CacheEntryMap<K, V>, CacheErr>>,
    },
    ValueMap {
        respond_to: oneshot::Sender<Result<HashMap<K, V>, CacheErr>>,
    },
    FindEntriesWhere {
        filter: QueryEntryFilter<K, V>,
        respond_to: oneshot::Sender<Result<Vec<CacheEntry<K, V>>, CacheErr>>,
    },
    FindWhere {
        filter: QueryValueFilter<V>,
        respond_to: oneshot::Sender<Result<Vec<V>, CacheErr>>,
    },
    FindOneEntryOptional {
        filter_name: &'static str,
        filter: QueryEntryFilter<K, V>,
        respond_to: oneshot::Sender<Result<Option<CacheEntry<K, V>>, CacheErr>>,
    },
    FindOneOptional {
        filter_name: &'static str,
        filter: QueryValueFilter<V>,
        respond_to: oneshot::Sender<Result<Option<V>, CacheErr>>,
    },
    FindOneEntry {
        filter_name: &'static str,
        filter: QueryEntryFilter<K, V>,
        respond_to: oneshot::Sender<Result<CacheEntry<K, V>, CacheErr>>,
    },
    FindOne {
        filter_name: &'static str,
        filter: QueryValueFilter<V>,
        respond_to: oneshot::Sender<Result<V, CacheErr>>,
    },
    GetDirtyEntries {
        respond_to: oneshot::Sender<Result<Vec<CacheEntry<K, V>>, CacheErr>>,
    },
}

// =================================== WORKER ====================================== //
pub struct Worker<SingleThreadCacheT, K, V>
where
    SingleThreadCacheT: SingleThreadCache<K, V>,
    K: ConcurrentCacheKey,
    V: ConcurrentCacheValue,
{
    pub cache: SingleThreadCacheT,
    pub receiver: Receiver<WorkerCommand<K, V>>,
}

impl<SingleThreadCacheT, K, V> Worker<SingleThreadCacheT, K, V>
where
    SingleThreadCacheT: SingleThreadCache<K, V>,
    K: ConcurrentCacheKey,
    V: ConcurrentCacheValue,
{
    pub async fn run(mut self) {
        while let Some(cmd) = self.receiver.recv().await {
            match cmd {
                WorkerCommand::Shutdown { respond_to } => {
                    if respond_to.send(Ok(())).is_err() {
                        error!("Actor failed to send shutdown response");
                    }
                    break;
                }
                WorkerCommand::ReadEntryOptional { key, respond_to } => {
                    let result = self.cache.read_entry_optional(&key).await;
                    if respond_to.send(result).is_err() {
                        error!("Actor failed to read optional cache entry");
                    }
                }
                WorkerCommand::ReadEntry { key, respond_to } => {
                    let result = self.cache.read_entry(&key).await;
                    if respond_to.send(result).is_err() {
                        error!("Actor failed to read cache entry");
                    }
                }
                WorkerCommand::ReadOptional { key, respond_to } => {
                    let result = self.cache.read_optional(&key).await;
                    if respond_to.send(result).is_err() {
                        error!("Actor failed to read optional cache entry");
                    }
                }
                WorkerCommand::Read { key, respond_to } => {
                    let result = self.cache.read(&key).await;
                    if respond_to.send(result).is_err() {
                        error!("Actor failed to read cache entry");
                    }
                }
                WorkerCommand::Write {
                    key,
                    value,
                    is_dirty,
                    overwrite,
                    respond_to,
                } => {
                    let result = self.cache.write(key, value, is_dirty, overwrite).await;
                    if respond_to.send(result).is_err() {
                        error!("Actor failed to write cache entry");
                    }
                }
                WorkerCommand::Delete { key, respond_to } => {
                    let result = self.cache.delete(&key).await;
                    if respond_to.send(result).is_err() {
                        error!("Actor failed to delete cache entry");
                    }
                }
                WorkerCommand::Prune { respond_to } => {
                    let result = self.cache.prune().await;
                    if respond_to.send(result).is_err() {
                        error!("Actor failed to prune cache");
                    }
                }
                WorkerCommand::Size { respond_to } => {
                    let result = self.cache.size().await;
                    if respond_to.send(result).is_err() {
                        error!("Actor failed to get cache size");
                    }
                }
                WorkerCommand::Entries { respond_to } => {
                    let result = self.cache.entries().await;
                    if respond_to.send(result).is_err() {
                        error!("Actor failed to get cache entries");
                    }
                }
                WorkerCommand::Values { respond_to } => {
                    let result = self.cache.values().await;
                    if respond_to.send(result).is_err() {
                        error!("Actor failed to get cache values");
                    }
                }
                WorkerCommand::EntryMap { respond_to } => {
                    let result = self.cache.entry_map().await;
                    if respond_to.send(result).is_err() {
                        error!("Actor failed to get cache entry map");
                    }
                }
                WorkerCommand::ValueMap { respond_to } => {
                    let result = self.cache.value_map().await;
                    if respond_to.send(result).is_err() {
                        error!("Actor failed to get cache value map");
                    }
                }
                WorkerCommand::FindEntriesWhere { filter, respond_to } => {
                    let result = self.cache.find_entries_where(filter).await;
                    if respond_to.send(result).is_err() {
                        error!("Actor failed to find all cache entries");
                    }
                }
                WorkerCommand::FindWhere { filter, respond_to } => {
                    let result = self.cache.find_where(filter).await;
                    if respond_to.send(result).is_err() {
                        error!("Actor failed to find all cache entries");
                    }
                }
                WorkerCommand::FindOneEntryOptional {
                    filter_name,
                    filter,
                    respond_to,
                } => {
                    let result = self
                        .cache
                        .find_one_entry_optional(filter_name, filter)
                        .await;
                    if respond_to.send(result).is_err() {
                        error!("Actor failed to find one cache entry");
                    }
                }
                WorkerCommand::FindOneOptional {
                    filter_name,
                    filter,
                    respond_to,
                } => {
                    let result = self.cache.find_one_optional(filter_name, filter).await;
                    if respond_to.send(result).is_err() {
                        error!("Actor failed to find one cache entry");
                    }
                }
                WorkerCommand::FindOneEntry {
                    filter_name,
                    filter,
                    respond_to,
                } => {
                    let result = self.cache.find_one_entry(filter_name, filter).await;
                    if respond_to.send(result).is_err() {
                        error!("Actor failed to find one cache entry");
                    }
                }
                WorkerCommand::FindOne {
                    filter_name,
                    filter,
                    respond_to,
                } => {
                    let result = self.cache.find_one(filter_name, filter).await;
                    if respond_to.send(result).is_err() {
                        error!("Actor failed to find one cache entry");
                    }
                }
                WorkerCommand::GetDirtyEntries { respond_to } => {
                    let result = self.cache.get_dirty_entries().await;
                    if respond_to.send(result).is_err() {
                        error!("Actor failed to get dirty entries");
                    }
                }
            }
        }
    }
}

// =============================== CONCURRENT CACHE ================================ //
#[derive(Debug)]
pub struct ConcurrentCache<SingleThreadCacheT, K, V>
where
    SingleThreadCacheT: SingleThreadCache<K, V>,
    K: ConcurrentCacheKey,
    V: ConcurrentCacheValue,
{
    sender: Sender<WorkerCommand<K, V>>,
    _phantom: std::marker::PhantomData<SingleThreadCacheT>,
}

impl<SingleThreadCacheT, K, V> ConcurrentCache<SingleThreadCacheT, K, V>
where
    SingleThreadCacheT: SingleThreadCache<K, V>,
    K: ConcurrentCacheKey,
    V: ConcurrentCacheValue,
{
    pub fn new(sender: Sender<WorkerCommand<K, V>>) -> Self {
        Self {
            sender,
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<SingleThreadCacheT, K, V> ConcurrentCache<SingleThreadCacheT, K, V>
where
    SingleThreadCacheT: SingleThreadCache<K, V>,
    K: ConcurrentCacheKey,
    V: ConcurrentCacheValue,
{
    async fn send_command<R>(
        &self,
        cmd: impl FnOnce(oneshot::Sender<R>) -> WorkerCommand<K, V>,
    ) -> Result<R, CacheErr> {
        let (send, recv) = oneshot::channel();
        self.sender.send(cmd(send)).await.map_err(|e| {
            CacheErr::SendActorMessageErr(SendActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })?;
        recv.await.map_err(|e| {
            CacheErr::ReceiveActorMessageErr(ReceiveActorMessageErr {
                source: Box::new(e),
                trace: trace!(),
            })
        })
    }

    pub async fn shutdown(&self) -> Result<(), CacheErr> {
        info!("Shutting down {} cache...", std::any::type_name::<V>());
        self.send_command(|tx| WorkerCommand::Shutdown { respond_to: tx })
            .await??;
        info!("{} cache shutdown complete", std::any::type_name::<V>());
        Ok(())
    }

    pub async fn read_entry_optional(&self, key: K) -> Result<Option<CacheEntry<K, V>>, CacheErr> {
        self.send_command(|tx| WorkerCommand::ReadEntryOptional {
            key,
            respond_to: tx,
        })
        .await?
    }

    pub async fn read_entry(&self, key: K) -> Result<CacheEntry<K, V>, CacheErr> {
        self.send_command(|tx| WorkerCommand::ReadEntry {
            key,
            respond_to: tx,
        })
        .await?
    }

    async fn read_optional_impl(&self, key: K) -> Result<Option<V>, CacheErr> {
        self.send_command(|tx| WorkerCommand::ReadOptional {
            key,
            respond_to: tx,
        })
        .await?
    }

    async fn read_impl(&self, key: K) -> Result<V, CacheErr> {
        self.send_command(|tx| WorkerCommand::Read {
            key,
            respond_to: tx,
        })
        .await?
    }

    pub async fn write<F>(
        &self,
        key: K,
        value: V,
        is_dirty: F,
        overwrite: Overwrite,
    ) -> Result<(), CacheErr>
    where
        F: Fn(Option<&CacheEntry<K, V>>, &V) -> bool + Send + Sync + 'static,
    {
        self.send_command(|tx| WorkerCommand::Write {
            key,
            value,
            is_dirty: Box::new(is_dirty),
            overwrite,
            respond_to: tx,
        })
        .await?
    }

    pub async fn delete(&self, key: K) -> Result<(), CacheErr> {
        self.send_command(|tx| WorkerCommand::Delete {
            key,
            respond_to: tx,
        })
        .await?
    }

    pub async fn prune(&self) -> Result<(), CacheErr> {
        self.send_command(|tx| WorkerCommand::Prune { respond_to: tx })
            .await?
    }

    pub async fn size(&self) -> Result<usize, CacheErr> {
        self.send_command(|tx| WorkerCommand::Size { respond_to: tx })
            .await?
    }

    pub async fn entries(&self) -> Result<Vec<CacheEntry<K, V>>, CacheErr> {
        self.send_command(|tx| WorkerCommand::Entries { respond_to: tx })
            .await?
    }

    pub async fn values(&self) -> Result<Vec<V>, CacheErr> {
        self.send_command(|tx| WorkerCommand::Values { respond_to: tx })
            .await?
    }

    pub async fn entry_map(&self) -> Result<HashMap<K, CacheEntry<K, V>>, CacheErr> {
        self.send_command(|tx| WorkerCommand::EntryMap { respond_to: tx })
            .await?
    }

    pub async fn value_map(&self) -> Result<HashMap<K, V>, CacheErr> {
        self.send_command(|tx| WorkerCommand::ValueMap { respond_to: tx })
            .await?
    }

    pub async fn find_entries_where<F>(&self, filter: F) -> Result<Vec<CacheEntry<K, V>>, CacheErr>
    where
        F: Fn(&CacheEntry<K, V>) -> bool + Send + Sync + 'static,
    {
        self.send_command(|tx| WorkerCommand::FindEntriesWhere {
            filter: Box::new(filter),
            respond_to: tx,
        })
        .await?
    }

    pub async fn find_one_entry_optional<F>(
        &self,
        filter_name: &'static str,
        filter: F,
    ) -> Result<Option<CacheEntry<K, V>>, CacheErr>
    where
        F: Fn(&CacheEntry<K, V>) -> bool + Send + Sync + 'static,
    {
        self.send_command(|tx| WorkerCommand::FindOneEntryOptional {
            filter_name,
            filter: Box::new(filter),
            respond_to: tx,
        })
        .await?
    }

    pub async fn find_one_entry<F>(
        &self,
        filter_name: &'static str,
        filter: F,
    ) -> Result<CacheEntry<K, V>, CacheErr>
    where
        F: Fn(&CacheEntry<K, V>) -> bool + Send + Sync + 'static,
    {
        self.send_command(|tx| WorkerCommand::FindOneEntry {
            filter_name,
            filter: Box::new(filter),
            respond_to: tx,
        })
        .await?
    }

    async fn find_where_impl<F>(&self, filter: F) -> Result<Vec<V>, CacheErr>
    where
        F: Fn(&V) -> bool + Send + Sync + 'static,
    {
        self.send_command(|tx| WorkerCommand::FindWhere {
            filter: Box::new(filter),
            respond_to: tx,
        })
        .await?
    }

    async fn find_one_optional_impl<F>(
        &self,
        filter_name: &'static str,
        filter: F,
    ) -> Result<Option<V>, CacheErr>
    where
        F: Fn(&V) -> bool + Send + Sync + 'static,
    {
        self.send_command(|tx| WorkerCommand::FindOneOptional {
            filter_name,
            filter: Box::new(filter),
            respond_to: tx,
        })
        .await?
    }

    async fn find_one_impl<F>(&self, filter_name: &'static str, filter: F) -> Result<V, CacheErr>
    where
        F: Fn(&V) -> bool + Send + Sync + 'static,
    {
        self.send_command(|tx| WorkerCommand::FindOne {
            filter_name,
            filter: Box::new(filter),
            respond_to: tx,
        })
        .await?
    }

    pub async fn get_dirty_entries(&self) -> Result<Vec<CacheEntry<K, V>>, CacheErr> {
        self.send_command(|tx| WorkerCommand::GetDirtyEntries { respond_to: tx })
            .await?
    }
}

// ==================================== FIND ======================================= //
impl<SingleThreadCacheT, K, V> Find<K, V> for ConcurrentCache<SingleThreadCacheT, K, V>
where
    SingleThreadCacheT: SingleThreadCache<K, V>,
    K: ConcurrentCacheKey,
    V: ConcurrentCacheValue,
{
    async fn find_where<F>(&self, filter: F) -> Result<Vec<V>, CrudErr>
    where
        F: Fn(&V) -> bool + Send + Sync + 'static,
    {
        self.find_where_impl(filter).await.map_err(CrudErr::from)
    }

    async fn find_one_optional<F>(
        &self,
        filter_name: &'static str,
        filter: F,
    ) -> Result<Option<V>, CrudErr>
    where
        F: Fn(&V) -> bool + Send + Sync + 'static,
    {
        self.find_one_optional_impl(filter_name, filter)
            .await
            .map_err(CrudErr::from)
    }

    async fn find_one<F>(&self, filter_name: &'static str, filter: F) -> Result<V, CrudErr>
    where
        F: Fn(&V) -> bool + Send + Sync + 'static,
    {
        self.find_one_impl(filter_name, filter)
            .await
            .map_err(CrudErr::from)
    }
}

// ==================================== READ ======================================= //
impl<SingleThreadCacheT, K, V> Read<K, V> for ConcurrentCache<SingleThreadCacheT, K, V>
where
    SingleThreadCacheT: SingleThreadCache<K, V>,
    K: ConcurrentCacheKey,
    V: ConcurrentCacheValue,
{
    async fn read(&self, key: K) -> Result<V, CrudErr> {
        self.read_impl(key).await.map_err(CrudErr::from)
    }

    async fn read_optional(&self, key: K) -> Result<Option<V>, CrudErr> {
        self.read_optional_impl(key).await.map_err(CrudErr::from)
    }
}
