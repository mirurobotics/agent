// standard library
use std::future::Future;
use std::sync::Arc;

// internal crates
use crate::activity;
use crate::authn::{
    token::Token,
    token_mngr::{TokenFile, TokenManager, TokenManagerExt},
};
use crate::cooldown;
use crate::crypt::jwt;
use crate::deploy::fsm;
use crate::filesys::path::PathExt;
use crate::http;
use crate::models::{
    device,
    device::{Device, DeviceStatus},
};
use crate::server::errors::*;
use crate::storage::{
    caches::{CacheCapacities, Caches},
    device::DeviceFile,
    layout::StorageLayout,
};
use crate::sync::syncer::{Syncer, SyncerArgs, SyncerExt};
use crate::trace;

// external crates
use tokio::task::JoinHandle;
use tracing::info;

pub type DeviceID = String;

#[derive(Clone, Debug)]
pub struct AppState {
    pub device_file: Arc<DeviceFile>,
    pub http_client: Arc<http::Client>,
    pub syncer: Arc<Syncer>,
    pub caches: Arc<Caches>,
    pub token_mngr: Arc<TokenManager>,
    pub activity_tracker: Arc<activity::Tracker>,
}

impl AppState {
    pub async fn init(
        agent_version: String,
        layout: &StorageLayout,
        cache_capacities: CacheCapacities,
        http_client: Arc<http::Client>,
        dpl_retry_policy: fsm::RetryPolicy,
    ) -> Result<(Self, impl Future<Output = ()>), ServerErr> {
        // storage layout stuff
        let auth_dir = layout.auth_dir();
        let private_key_file = auth_dir.private_key_file();
        private_key_file.assert_exists()?;

        let token_file =
            TokenFile::new_with_default(auth_dir.token_file(), Token::default()).await?;

        // get the device id
        let device_id = Self::init_device_id(layout, &token_file).await?;

        let (device_file, device_file_handle) =
            Self::init_device_file(layout, device_id.clone()).await?;
        let device_file = Arc::new(device_file);

        // initialize the caches
        let (caches, caches_shutdown_handle) = Caches::init(layout, cache_capacities).await?;
        let caches = Arc::new(caches);

        // initialize the token manager
        let (token_mngr, token_mngr_handle) = TokenManager::spawn(
            64,
            device_id.clone(),
            http_client.clone(),
            token_file,
            private_key_file,
        )?;
        let token_mngr = Arc::new(token_mngr);

        // initialize the syncer
        let (syncer, syncer_handle) = Syncer::spawn(
            64,
            SyncerArgs {
                device_id: device_id.clone(),
                device_file: device_file.clone(),
                http_client: http_client.clone(),
                token_mngr: token_mngr.clone(),
                cfg_inst_cache: caches.cfg_inst.clone(),
                cfg_inst_content_cache: caches.cfg_inst_content.clone(),
                deployment_cache: caches.deployment.clone(),
                deployment_dir: layout.config_instance_deployment_dir(),
                staging_dir: layout.temp_dir(),
                dpl_retry_policy,
                agent_version,
                backoff: cooldown::Backoff {
                    base_secs: 1,
                    growth_factor: 2,
                    max_secs: 12 * 60 * 60, // 12 hours
                },
            },
        )?;
        let syncer = Arc::new(syncer);

        // initialize the activity tracker
        let activity_tracker = Arc::new(activity::Tracker::new());

        let shutdown_handle = async move {
            let handles = vec![token_mngr_handle, syncer_handle, device_file_handle];

            futures::future::join(futures::future::join_all(handles), caches_shutdown_handle).await;
        };

        Ok((
            AppState {
                device_file,
                http_client,
                syncer,
                caches,
                token_mngr,
                activity_tracker,
            },
            shutdown_handle,
        ))
    }

    async fn init_device_id(
        layout: &StorageLayout,
        token_file: &TokenFile,
    ) -> Result<DeviceID, ServerErr> {
        // attempt to get the device id from the agent file
        let device_file_err = match layout.device_file().read_json::<Device>().await {
            Ok(device) => {
                return Ok(device.id.clone());
            }
            Err(e) => e,
        };

        // attempt to get the device id from the existing token on file
        let token = token_file.read().await;
        let device_id = match jwt::extract_device_id(&token.token) {
            Ok(device_id) => device_id,
            Err(e) => {
                return Err(ServerErr::MissingDeviceIDErr(Box::new(
                    MissingDeviceIDErr {
                        device_file_err,
                        jwt_err: e,
                        trace: trace!(),
                    },
                )));
            }
        };

        Ok(device_id)
    }

    async fn init_device_file(
        layout: &StorageLayout,
        device_id: String,
    ) -> Result<(DeviceFile, JoinHandle<()>), ServerErr> {
        // initialize the device file with some reasonable defaults
        let (device_file, device_file_handle) = DeviceFile::spawn_with_default(
            64,
            layout.device_file(),
            Device {
                id: device_id.clone(),
                activated: true,
                status: DeviceStatus::Offline,
                ..Device::default()
            },
        )
        .await?;

        device_file
            .patch(device::Updates {
                status: Some(DeviceStatus::Offline),
                ..device::Updates::empty()
            })
            .await?;
        Ok((device_file, device_file_handle))
    }

    pub async fn shutdown(&self) -> Result<(), ServerErr> {
        self.shutdown_device_file().await?;

        // shutdown the syncer
        self.syncer.shutdown().await?;

        // shutdown the caches
        self.caches.shutdown().await?;

        // shutdown the token manager
        self.token_mngr.shutdown().await?;

        Ok(())
    }

    async fn shutdown_device_file(&self) -> Result<(), ServerErr> {
        let device = self.device_file.read().await?;

        // if the last known device status was online, we'll set it to be offline
        match device.status {
            device::DeviceStatus::Online => {
                info!("Shutting down device file, setting device to offline");
                self.device_file
                    .patch(device::Updates::disconnected())
                    .await?;
            }
            device::DeviceStatus::Offline => {
                info!("Shutting down device file, device is already offline");
            }
        }
        self.device_file.shutdown().await.map_err(ServerErr::from)
    }
}
