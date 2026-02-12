// standard library
use std::future::Future;
use std::sync::Arc;

// internal crates
use crate::storage::config_instances::{ConfigInstanceCache, ConfigInstanceContentCache};
use crate::storage::errors::*;
use crate::storage::layout::StorageLayout;
use crate::trace;

#[derive(Copy, Clone, Debug)]
pub struct CacheCapacities {
    pub cfg_inst: usize,
    pub cfg_inst_content: usize,
}

impl Default for CacheCapacities {
    fn default() -> Self {
        Self {
            cfg_inst: 100,
            cfg_inst_content: 100,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Caches {
    pub cfg_inst: Arc<ConfigInstanceCache>,
    pub cfg_inst_content: Arc<ConfigInstanceContentCache>,
}

impl Caches {
    pub async fn init(
        layout: &StorageLayout,
        capacities: CacheCapacities,
    ) -> Result<(Caches, impl Future<Output = ()>), StorageErr> {
        // config instance
        let (cfg_inst_cache, cfg_inst_cache_handle) =
            ConfigInstanceCache::spawn(64, layout.config_instance_cache(), capacities.cfg_inst)
                .await
                .map_err(|e| {
                    StorageErr::CacheErr(Box::new(StorageCacheErr {
                        source: e,
                        trace: trace!(),
                    }))
                })?;
        let cfg_inst_cache = Arc::new(cfg_inst_cache);

        // config instance content
        let (cfg_inst_content_cache, cfg_inst_content_cache_handle) =
            ConfigInstanceContentCache::spawn(
                64,
                layout.config_instance_content_cache(),
                capacities.cfg_inst_content,
            )
            .await
            .map_err(|e| {
                StorageErr::CacheErr(Box::new(StorageCacheErr {
                    source: e,
                    trace: trace!(),
                }))
            })?;
        let cfg_inst_content_cache = Arc::new(cfg_inst_content_cache);

        // return the shutdown handler
        let shutdown_handle = async move {
            let handles = vec![
                cfg_inst_cache_handle,
                cfg_inst_content_cache_handle,
            ];

            futures::future::join_all(handles).await;
        };

        Ok((
            Caches {
                cfg_inst: cfg_inst_cache,
                cfg_inst_content: cfg_inst_content_cache,
            },
            shutdown_handle,
        ))
    }

    pub async fn shutdown(&self) -> Result<(), StorageErr> {
        self.cfg_inst.shutdown().await.map_err(|e| {
            StorageErr::CacheErr(Box::new(StorageCacheErr {
                source: e,
                trace: trace!(),
            }))
        })?;
        self.cfg_inst_content.shutdown().await.map_err(|e| {
            StorageErr::CacheErr(Box::new(StorageCacheErr {
                source: e,
                trace: trace!(),
            }))
        })?;

        Ok(())
    }
}
