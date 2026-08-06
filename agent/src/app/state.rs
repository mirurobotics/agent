// standard crates
use std::future::Future;
use std::sync::Arc;

// internal crates
use crate::activity;
use crate::authn::{self, token_mngr::TokenFile, TokenManagerExt};
use crate::cooldown;
use crate::deploy::{apply, fsm};
use crate::disk;
use crate::events;
use crate::filesys::PathExt;
use crate::http;
use crate::scan::{self, state::ScanSnapshotFile, ScannerArgs, ScannerExt};
use crate::server;
use crate::sync::{self, syncer::SyncerArgs, SyncerExt};
use crate::upload::{self, UploaderExt};

#[derive(Clone, Debug)]
pub struct AppState {
    pub storage: Arc<disk::Storage>,
    pub http_client: Arc<http::Client>,
    pub syncer: Arc<sync::Syncer>,
    pub scanner: Option<Arc<scan::Scanner>>,
    pub uploader: Option<Arc<upload::Uploader>>,
    pub token_mngr: Arc<authn::TokenManager>,
    pub activity_tracker: Arc<activity::Tracker>,
    pub event_hub: events::EventHub,
}

impl AppState {
    pub async fn init(
        layout: &disk::Layout,
        capacities: disk::Capacities,
        http_client: Arc<http::Client>,
        dpl_retry_policy: fsm::RetryPolicy,
        enable_uploader: bool,
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
        let device_id = disk::resolve_device_id(layout).await?;

        // initialize storage
        let (stor, storage_handle) = disk::Storage::init(layout, capacities, device_id).await?;
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
            },
        )?;
        let syncer = Arc::new(syncer);

        // initialize the activity tracker
        let activity_tracker = Arc::new(activity::Tracker::new());

        // initialize the scanner (optional)
        let (scanner, scanner_handle) = Self::init_scanner(layout, enable_uploader).await;

        // initialize the uploader (optional)
        let (uploader, uploader_handle) = Self::init_uploader(
            layout,
            enable_uploader,
            http_client.clone(),
            token_mngr.clone(),
        )
        .await;

        let shutdown_handle = async move {
            let mut handles = vec![token_mngr_handle, syncer_handle, event_hub_handle];
            if let Some(handle) = scanner_handle {
                handles.push(handle);
            }
            if let Some(handle) = uploader_handle {
                handles.push(handle);
            }

            futures::future::join(futures::future::join_all(handles), storage_handle).await;
        };

        Ok((
            AppState {
                storage,
                http_client,
                syncer,
                scanner,
                uploader,
                token_mngr,
                activity_tracker,
                event_hub,
            },
            shutdown_handle,
        ))
    }

    /// Spawn the scanner actor with an on-disk snapshot. Fail-open by design:
    /// a snapshot-file error degrades to scanning without persistence, and a
    /// spawn error degrades to no scanner at all — the agent must boot even
    /// when the scanner cannot.
    async fn init_scanner(
        layout: &disk::Layout,
        enable_scanner: bool,
    ) -> (
        Option<Arc<scan::Scanner>>,
        Option<tokio::task::JoinHandle<()>>,
    ) {
        if !enable_scanner {
            return (None, None);
        }

        let snapshot_file = match ScanSnapshotFile::new_with_default(
            layout.scanner_snapshot(),
            Default::default(),
        )
        .await
        {
            Ok(file) => Some(file),
            Err(e) => {
                tracing::error!(
                        "failed to initialize scanner snapshot file; scanning will run without persistence: {e}"
                    );
                None
            }
        };

        let args = ScannerArgs {
            snapshot_file,
            ..ScannerArgs::default()
        };
        match scan::Scanner::spawn(64, args) {
            Ok((scanner, handle)) => (Some(Arc::new(scanner)), Some(handle)),
            Err(e) => {
                tracing::error!("failed to spawn scanner; continuing without scanning: {e}");
                (None, None)
            }
        }
    }

    /// Spawn the uploader actor driving the live executor (credential mint → native SDK
    /// transfer → confirm). Fail-open by design: a snapshot-file error degrades to
    /// uploading without queue persistence, and a spawn error degrades to no uploader
    /// — the agent must boot even when the uploader cannot.
    async fn init_uploader(
        layout: &disk::Layout,
        enable_scanner: bool,
        http_client: Arc<http::Client>,
        token_mngr: Arc<authn::TokenManager>,
    ) -> (
        Option<Arc<upload::Uploader>>,
        Option<tokio::task::JoinHandle<()>>,
    ) {
        if !enable_scanner {
            return (None, None);
        }

        let snapshot_file = match upload::QueueSnapshotFile::new_with_default(
            layout.upload_queue(),
            Default::default(),
        )
        .await
        {
            Ok(file) => Some(file),
            Err(e) => {
                tracing::error!(
                    "failed to initialize upload queue snapshot file; uploads will run without persistence: {e}"
                );
                None
            }
        };

        let executor = Arc::new(upload::LiveExecutor::new(
            http_client,
            token_mngr,
            upload::SdkTransfer::default(),
            // the deleter is wired in with the delete worker (Milestone 5);
            // until then confirmed uploads skip deletion with a warning.
            None::<Arc<crate::delete::Deleter>>,
        ));
        match upload::Uploader::spawn(
            64,
            executor,
            upload::UploaderOptions::default(),
            snapshot_file,
            |wait| tokio::time::sleep(wait),
        ) {
            Ok((uploader, handle)) => (Some(Arc::new(uploader)), Some(handle)),
            Err(e) => {
                tracing::error!("failed to spawn uploader; continuing without uploads: {e}");
                (None, None)
            }
        }
    }

    pub async fn shutdown(&self) -> Result<(), server::ServerErr> {
        let mut first_err: Option<server::ServerErr> = None;

        // shutdown the uploader first: it depends on nothing else in the app, and its
        // feeder (the scan-upload bridge worker) has already been joined by this point.
        if let Some(uploader) = &self.uploader {
            if let Err(e) = uploader.shutdown().await {
                tracing::error!("failed to shutdown uploader: {e}");
                first_err.get_or_insert(e.into());
            }
        }

        // shutdown the scanner before the syncer (it uses syncer to determine the
        // correct set of rules to use for scanning)
        if let Some(scanner) = &self.scanner {
            if let Err(e) = scanner.shutdown().await {
                tracing::error!("failed to shutdown scanner: {e}");
                first_err.get_or_insert(e.into());
            }
        }

        // shutdown the syncer before storage (it uses storage during sync)
        if let Err(e) = self.syncer.shutdown().await {
            tracing::error!("failed to shutdown syncer: {e}");
            first_err.get_or_insert(e.into());
        }

        if let Err(e) = self.event_hub.shutdown().await {
            tracing::error!("failed to shutdown event hub: {e}");
            first_err.get_or_insert(e.into());
        }

        if let Err(e) = self.storage.shutdown().await {
            tracing::error!("failed to shutdown storage: {e}");
            first_err.get_or_insert(e.into());
        }

        if let Err(e) = self.token_mngr.shutdown().await {
            tracing::error!("failed to shutdown token manager: {e}");
            first_err.get_or_insert(e.into());
        }

        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}
