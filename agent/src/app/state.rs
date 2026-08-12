// standard crates
use std::future::Future;
use std::sync::Arc;

// internal crates
use crate::activity;
use crate::authn::{self, token_mngr::TokenFile, TokenManagerExt};
use crate::cooldown;
use crate::delete::{self, DeleterExt};
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
    pub scanner: Arc<scan::Scanner>,
    pub deleter: Arc<delete::Deleter>,
    pub uploader: Arc<upload::Uploader>,
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

        // initialize the deleter before the uploader: the uploader's executor
        // enqueues pending deletions onto it
        let (deleter, deleter_handle) = Self::init_deleter(layout).await?;

        // initialize the uploader before the scanner: the scanner's
        // stable-file sinks are built from the uploader handle
        let (uploader, uploader_handle) = Self::init_uploader(
            layout,
            http_client.clone(),
            token_mngr.clone(),
            deleter.clone(),
        )
        .await?;

        // initialize the scanner with the upload sink
        let sinks: Vec<Arc<dyn scan::StableFileSink>> = vec![Arc::new(
            upload::UploadStableFileSink::new(uploader.clone()),
        )];
        let (scanner, scanner_handle) = Self::init_scanner(layout, sinks).await?;

        let shutdown_handle = async move {
            let handles = vec![
                token_mngr_handle,
                syncer_handle,
                event_hub_handle,
                scanner_handle,
                deleter_handle,
                uploader_handle,
            ];

            futures::future::join(futures::future::join_all(handles), storage_handle).await;
        };

        Ok((
            AppState {
                storage,
                http_client,
                syncer,
                scanner,
                deleter,
                uploader,
                token_mngr,
                activity_tracker,
                event_hub,
            },
            shutdown_handle,
        ))
    }

    /// Spawn the scanner actor with an on-disk snapshot. A snapshot-file error
    /// degrades to scanning without persistence (fail-open — a bad disk must
    /// not brick config deployment); a spawn error fails boot, but no such
    /// error path currently exists (`Scanner::spawn` cannot fail today).
    async fn init_scanner(
        layout: &disk::Layout,
        sinks: Vec<Arc<dyn scan::StableFileSink>>,
    ) -> Result<(Arc<scan::Scanner>, tokio::task::JoinHandle<()>), server::ServerErr> {
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
            sinks,
            ..ScannerArgs::default()
        };
        let (scanner, handle) = scan::Scanner::spawn(64, args)?;
        Ok((Arc::new(scanner), handle))
    }

    /// Spawn the deleter actor with an on-disk snapshot. A snapshot-file error
    /// degrades to deleting without queue persistence (fail-open); a spawn
    /// error fails boot, but no such error path currently exists
    /// (`Deleter::spawn` cannot fail today).
    async fn init_deleter(
        layout: &disk::Layout,
    ) -> Result<(Arc<delete::Deleter>, tokio::task::JoinHandle<()>), server::ServerErr> {
        let snapshot_file = match delete::DeleteQueueSnapshotFile::new_with_default(
            layout.delete_queue(),
            Default::default(),
        )
        .await
        {
            Ok(file) => Some(file),
            Err(e) => {
                tracing::error!(
                    "failed to initialize delete queue snapshot file; deletions will run without persistence: {e}"
                );
                None
            }
        };

        let args = delete::DeleterArgs {
            snapshot_file,
            ..delete::DeleterArgs::default()
        };
        let (deleter, handle) = delete::Deleter::spawn(64, args)?;
        Ok((Arc::new(deleter), handle))
    }

    /// Spawn the uploader actor driving the live executor (credential mint → native SDK
    /// transfer → confirm). A snapshot-file error degrades to uploading without
    /// queue persistence (fail-open); a spawn error fails boot, but no such
    /// error path currently exists (`Uploader::spawn` cannot fail today).
    async fn init_uploader(
        layout: &disk::Layout,
        http_client: Arc<http::Client>,
        token_mngr: Arc<authn::TokenManager>,
        deleter: Arc<delete::Deleter>,
    ) -> Result<(Arc<upload::Uploader>, tokio::task::JoinHandle<()>), server::ServerErr> {
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
            Some(deleter),
        ));
        let (uploader, handle) = upload::Uploader::spawn(
            64,
            executor,
            upload::UploaderOptions::default(),
            snapshot_file,
            |wait| tokio::time::sleep(wait),
            chrono::Utc::now,
        )?;
        Ok((Arc::new(uploader), handle))
    }

    pub async fn shutdown(&self) -> Result<(), server::ServerErr> {
        let mut first_err: Option<server::ServerErr> = None;

        // shutdown the uploader first: it depends on nothing else in the app. The
        // scanner's upload sink may still enqueue into the stopped uploader if a
        // scan tick races this shutdown; the sink logs a warn and the tick
        // completes, so ordering stays safe.
        if let Err(e) = self.uploader.shutdown().await {
            tracing::error!("failed to shutdown uploader: {e}");
            first_err.get_or_insert(e.into());
        }

        // shutdown the deleter after the uploader (whose executor is its producer);
        // the delete driver worker has already been joined by this point
        if let Err(e) = self.deleter.shutdown().await {
            tracing::error!("failed to shutdown deleter: {e}");
            first_err.get_or_insert(e.into());
        }

        // shutdown the scanner before the syncer (it uses syncer to determine the
        // correct set of rules to use for scanning)
        if let Err(e) = self.scanner.shutdown().await {
            tracing::error!("failed to shutdown scanner: {e}");
            first_err.get_or_insert(e.into());
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
