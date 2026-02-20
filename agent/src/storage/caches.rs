// standard library
use std::future::Future;
use std::sync::Arc;

// internal crates
use super::config_instances::{CfgInstContent, CfgInsts};
use super::deployments::Deployments;
use super::errors::*;
use super::layout::Layout;

#[derive(Copy, Clone, Debug)]
pub struct Capacities {
    pub cfg_inst: usize,
    pub cfg_inst_content: usize,
    pub deployment: usize,
}

impl Default for Capacities {
    fn default() -> Self {
        Self {
            cfg_inst: 100,
            cfg_inst_content: 100,
            deployment: 100,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Caches {
    pub cfg_inst: Arc<CfgInsts>,
    pub cfg_inst_content: Arc<CfgInstContent>,
    pub deployment: Arc<Deployments>,
}

impl Caches {
    pub async fn init(
        layout: &Layout,
        capacities: Capacities,
    ) -> Result<(Caches, impl Future<Output = ()>), StorageErr> {
        // config instance
        let (cfg_inst_cache, cfg_inst_cache_handle) =
            CfgInsts::spawn(64, layout.config_instance_cache(), capacities.cfg_inst).await?;
        let cfg_inst_cache = Arc::new(cfg_inst_cache);

        // config instance content
        let (cfg_inst_content_cache, cfg_inst_content_cache_handle) = CfgInstContent::spawn(
            64,
            layout.config_instance_content_cache(),
            capacities.cfg_inst_content,
        )
        .await?;
        let cfg_inst_content_cache = Arc::new(cfg_inst_content_cache);

        // deployment
        let (deployment_cache, deployment_cache_handle) =
            Deployments::spawn(64, layout.deployment_cache(), capacities.deployment).await?;
        let deployment_cache = Arc::new(deployment_cache);

        // return the shutdown handler
        let shutdown_handle = async move {
            let handles = vec![
                cfg_inst_cache_handle,
                cfg_inst_content_cache_handle,
                deployment_cache_handle,
            ];

            futures::future::join_all(handles).await;
        };

        Ok((
            Caches {
                cfg_inst: cfg_inst_cache,
                cfg_inst_content: cfg_inst_content_cache,
                deployment: deployment_cache,
            },
            shutdown_handle,
        ))
    }

    pub async fn shutdown(&self) -> Result<(), StorageErr> {
        self.cfg_inst.shutdown().await?;
        self.cfg_inst_content.shutdown().await?;
        self.deployment.shutdown().await?;

        Ok(())
    }
}
