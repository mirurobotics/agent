// standard crates
use std::future::Future;
use std::sync::Arc;

// internal crates
use crate::activity;
use crate::authn::{self, token_mngr::TokenFile, TokenManagerExt};
use crate::cooldown;
use crate::deploy::{apply, fsm};
use crate::events;
use crate::filesys::PathExt;
use crate::http;
use crate::server;
use crate::storage;
use crate::sync::{self, syncer::SyncerArgs, SyncerExt};
use crate::upload::{self, UploaderExt};

#[derive(Clone, Debug)]
pub struct AppState {
    pub storage: Arc<storage::Storage>,
    pub http_client: Arc<http::Client>,
    pub syncer: Arc<sync::Syncer>,
    pub uploader: Arc<upload::Uploader>,
    pub token_mngr: Arc<authn::TokenManager>,
    pub activity_tracker: Arc<activity::Tracker>,
    pub event_hub: events::EventHub,
}

impl AppState {
    pub async fn init(
        layout: &storage::Layout,
        capacities: storage::Capacities,
        http_client: Arc<http::Client>,
        dpl_retry_policy: fsm::RetryPolicy,
        uploader_opts: upload::Options,
    ) -> Result<(Self, impl Future<Output = ()>), server::ServerErr> {
        // storage layout stuff
        let auth_dir = layout.auth();
        let private_key_file = auth_dir.private_key();
        private_key_file.assert_exists()?;
        let public_key_file = auth_dir.public_key();
        public_key_file.assert_exists()?;

        let token_file =
            TokenFile::new_with_default(auth_dir.token(), authn::Token::default()).await?;

        // get the device id
        let device_id = storage::resolve_device_id(layout).await?;

        // initialize storage
        let (stor, storage_handle) = storage::Storage::init(layout, capacities, device_id).await?;
        let storage = Arc::new(stor);

        // initialize the token manager
        let (token_mngr, token_mngr_handle) = authn::TokenManager::spawn(
            64,
            http_client.clone(),
            token_file,
            private_key_file,
            public_key_file,
        )?;
        let token_mngr = Arc::new(token_mngr);

        // initialize the event hub
        let (event_hub, event_hub_handle) =
            events::EventHub::spawn(layout.events_log_file(), Default::default()).await?;

        // initialize the uploader before the syncer, which pushes the active
        // upload-rule set into it after each sync
        let (uploader, uploader_handle) = upload::Uploader::spawn(64, uploader_opts)?;
        let uploader = Arc::new(uploader);

        // initialize the syncer
        let (syncer, syncer_handle) = sync::Syncer::spawn(
            64,
            SyncerArgs {
                storage: storage.clone(),
                http_client: http_client.clone(),
                token_mngr: token_mngr.clone(),
                deploy_opts: apply::DeployOpts {
                    retry_policy: dpl_retry_policy,
                },
                backoff: cooldown::Backoff {
                    base_secs: 1,
                    growth_factor: 2,
                    max_secs: 12 * 60 * 60, // 12 hours
                },
                event_hub: event_hub.clone(),
                uploader: uploader.clone(),
            },
        )?;
        let syncer = Arc::new(syncer);

        // initialize the activity tracker
        let activity_tracker = Arc::new(activity::Tracker::new());

        let shutdown_handle = async move {
            let handles = vec![
                token_mngr_handle,
                syncer_handle,
                uploader_handle,
                event_hub_handle,
            ];

            futures::future::join(futures::future::join_all(handles), storage_handle).await;
        };

        Ok((
            AppState {
                storage,
                http_client,
                syncer,
                uploader,
                token_mngr,
                activity_tracker,
                event_hub,
            },
            shutdown_handle,
        ))
    }

    pub async fn shutdown(&self) -> Result<(), server::ServerErr> {
        // shutdown the syncer first (it uses storage during sync and pushes
        // rules into the uploader)
        self.syncer.shutdown().await?;

        // shutdown the uploader
        if let Err(e) = self.uploader.shutdown().await {
            tracing::error!("failed to shutdown uploader: {e}");
        }

        // shutdown the event hub
        if let Err(e) = self.event_hub.shutdown().await {
            tracing::error!("failed to shutdown event hub: {e}");
        }

        // shutdown storage (sets device offline + shuts down all stores)
        self.storage.shutdown().await?;

        // shutdown the token manager
        self.token_mngr.shutdown().await?;

        Ok(())
    }
}
