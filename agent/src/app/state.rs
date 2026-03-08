// standard crates
use std::future::Future;
use std::sync::Arc;

// internal crates
use crate::activity;
use crate::authn::{self, token_mngr::TokenFile, TokenManagerExt};
use crate::cooldown;
use crate::crypt::jwt;
use crate::deploy::{apply, fsm};
use crate::filesys::PathExt;
use crate::http;
use crate::models;
use crate::server::{self, errors::MissingDeviceIDErr};
use crate::storage;
use crate::sync::{self, syncer::SyncerArgs, SyncerExt};
use crate::trace;

pub type DeviceID = String;

#[derive(Clone, Debug)]
pub struct AppState {
    pub storage: Arc<storage::Storage>,
    pub http_client: Arc<http::Client>,
    pub syncer: Arc<sync::Syncer>,
    pub token_mngr: Arc<authn::TokenManager>,
    pub activity_tracker: Arc<activity::Tracker>,
}

impl AppState {
    pub async fn init(
        agent_version: String,
        layout: &storage::Layout,
        capacities: storage::Capacities,
        http_client: Arc<http::Client>,
        dpl_retry_policy: fsm::RetryPolicy,
    ) -> Result<(Self, impl Future<Output = ()>), server::ServerErr> {
        // storage layout stuff
        let auth_dir = layout.auth();
        let private_key_file = auth_dir.private_key();
        private_key_file.assert_exists()?;

        let token_file =
            TokenFile::new_with_default(auth_dir.token(), authn::Token::default()).await?;

        // get the device id
        let device_id = Self::init_device_id(layout, &token_file).await?;

        // initialize storage
        let (stor, storage_handle) =
            storage::Storage::init(layout, capacities, device_id.clone()).await?;
        let storage = Arc::new(stor);

        // initialize the token manager
        let (token_mngr, token_mngr_handle) = authn::TokenManager::spawn(
            64,
            device_id.clone(),
            http_client.clone(),
            token_file,
            private_key_file,
        )?;
        let token_mngr = Arc::new(token_mngr);

        // initialize the syncer
        let deploy_target_dir = layout.customer_configs();
        let deploy_staging_dir = layout.srv_temp_dir();
        let (syncer, syncer_handle) = sync::Syncer::spawn(
            64,
            SyncerArgs {
                storage: storage.clone(),
                http_client: http_client.clone(),
                token_mngr: token_mngr.clone(),
                deploy_opts: apply::DeployOpts {
                    staging_dir: deploy_staging_dir,
                    target_dir: deploy_target_dir,
                    retry_policy: dpl_retry_policy,
                },
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
            let handles = vec![token_mngr_handle, syncer_handle];

            futures::future::join(futures::future::join_all(handles), storage_handle).await;
        };

        Ok((
            AppState {
                storage,
                http_client,
                syncer,
                token_mngr,
                activity_tracker,
            },
            shutdown_handle,
        ))
    }

    async fn init_device_id(
        layout: &storage::Layout,
        token_file: &TokenFile,
    ) -> Result<DeviceID, server::ServerErr> {
        // attempt to get the device id from the agent file
        let device_file_err = match layout.device().read_json::<models::Device>().await {
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
                return Err(server::ServerErr::MissingDeviceIDErr(Box::new(
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

    pub async fn shutdown(&self) -> Result<(), server::ServerErr> {
        // shutdown the syncer first (it uses storage during sync)
        self.syncer.shutdown().await?;

        // shutdown storage (sets device offline + shuts down all stores)
        self.storage.shutdown().await?;

        // shutdown the token manager
        self.token_mngr.shutdown().await?;

        Ok(())
    }
}
