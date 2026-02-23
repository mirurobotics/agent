// standard library
use std::future::Future;
use std::sync::Arc;

pub mod config_instances;
pub mod deployments;
pub mod device;
pub mod errors;
pub mod layout;
pub mod settings;
pub mod setup;

pub use self::config_instances::{CfgInstContent, CfgInsts};
pub use self::deployments::{Deployments, DplEntry};
pub use self::device::{assert_activated, Device};
pub use self::errors::{DeviceNotActivatedErr, StorageErr};
pub use self::layout::Layout;
pub use self::settings::{Backend, MQTTBroker, Settings};

use self::device::Device as DeviceStorage;
use self::errors::StorageErr as StorErr;
use self::layout::Layout as StorLayout;
use crate::models;

use tracing::info;

#[derive(Copy, Clone, Debug)]
pub struct Capacities {
    pub cfg_insts: usize,
    pub cfg_inst_content: usize,
    pub deployments: usize,
}

impl Default for Capacities {
    fn default() -> Self {
        Self {
            cfg_insts: 100,
            cfg_inst_content: 100,
            deployments: 100,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CfgInstStor {
    pub meta: Arc<CfgInsts>,
    pub content: Arc<CfgInstContent>,
}

pub struct CfgInstRef<'a> {
    pub meta: &'a CfgInsts,
    pub content: &'a CfgInstContent,
}

impl CfgInstStor {
    pub fn as_ref(&self) -> CfgInstRef<'_> {
        CfgInstRef {
            meta: &self.meta,
            content: &self.content,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Storage {
    pub device: Arc<DeviceStorage>,
    pub cfg_insts: CfgInstStor,
    pub deployments: Arc<Deployments>,
}

impl Storage {
    pub async fn init(
        layout: &StorLayout,
        capacities: Capacities,
        device_id: String,
    ) -> Result<(Storage, impl Future<Output = ()>), StorErr> {
        // device storage
        let (device_storage, device_storage_handle) = DeviceStorage::spawn_with_default(
            64,
            layout.device(),
            models::Device {
                id: device_id.clone(),
                activated: true,
                status: models::DeviceStatus::Offline,
                ..models::Device::default()
            },
        )
        .await?;

        device_storage
            .patch(models::device::Updates {
                status: Some(models::DeviceStatus::Offline),
                ..models::device::Updates::empty()
            })
            .await?;

        let device = Arc::new(device_storage);

        // config instance metadata
        let (cfg_inst_stor, cfg_inst_stor_handle) =
            CfgInsts::spawn(64, layout.config_instance_meta(), capacities.cfg_insts).await?;
        let cfg_inst_metadata = Arc::new(cfg_inst_stor);

        // config instance content
        let (cfg_inst_content_stor, cfg_inst_content_stor_handle) = CfgInstContent::spawn(
            64,
            layout.config_instance_content(),
            capacities.cfg_inst_content,
        )
        .await?;
        let cfg_inst_content = Arc::new(cfg_inst_content_stor);

        // deployments
        let (deployment_stor, deployment_stor_handle) =
            Deployments::spawn(64, layout.deployments(), capacities.deployments).await?;
        let deployments = Arc::new(deployment_stor);

        let shutdown_handle = async move {
            let handles = vec![
                device_storage_handle,
                cfg_inst_stor_handle,
                cfg_inst_content_stor_handle,
                deployment_stor_handle,
            ];

            futures::future::join_all(handles).await;
        };

        Ok((
            Storage {
                device,
                cfg_insts: CfgInstStor {
                    meta: cfg_inst_metadata,
                    content: cfg_inst_content,
                },
                deployments,
            },
            shutdown_handle,
        ))
    }

    pub async fn shutdown(&self) -> Result<(), StorErr> {
        // if the device is online, set it to offline before shutting down
        let device_data = self.device.read().await?;
        match device_data.status {
            models::DeviceStatus::Online => {
                info!("Shutting down device storage, setting device to offline");
                self.device
                    .patch(models::device::Updates::disconnected())
                    .await?;
            }
            models::DeviceStatus::Offline => {
                info!("Shutting down device storage, device is already offline");
            }
        }

        self.device.shutdown().await?;
        self.cfg_insts.meta.shutdown().await?;
        self.cfg_insts.content.shutdown().await?;
        self.deployments.shutdown().await?;

        Ok(())
    }
}
