// standard crates
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

// internal crates
use crate::activity;
use crate::app::{
    options::{AppOptions, LifecycleOptions},
    state::AppState,
};
use crate::authn::{self, TokenManagerExt};
use crate::http;
use crate::scan::{self, ScanSnapshotFile, ScannerArgs, ScannerExt};
use crate::server::{self, errors::*, serve::serve};
use crate::trace;
use crate::workers::{
    mqtt, poller, scan_bridge,
    token_refresh::{run_token_refresh_worker, TokenRefreshWorkerOptions},
};

// external crates
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::{error, info};

pub async fn run(
    options: AppOptions,
    shutdown_signal: impl Future<Output = ()> + Send + 'static,
) -> Result<(), ServerErr> {
    info!("Initializing miru agent...");

    // Create a single shutdown channel that all components will listen to
    let (shutdown_tx, _shutdown_rx): (tokio::sync::broadcast::Sender<()>, _) =
        tokio::sync::broadcast::channel(1);
    let mut shutdown_manager = ShutdownManager::new(shutdown_tx.clone(), options.lifecycle);

    // initialize the app (and shutdown if failures occur)
    let app_state = match init(&options, shutdown_tx.clone(), &mut shutdown_manager).await {
        Ok(state) => state,
        Err(e) => {
            error!("Failed to start server: {}", e);
            shutdown_manager.shutdown().await?;
            return Err(e);
        }
    };

    // if the app is not persistent, wait for ctrl-c, an idle timeout, or max runtime
    // reached to trigger a shutdown
    if !options.lifecycle.is_persistent {
        tokio::select! {
            _ = shutdown_signal => {
                info!("Shutdown signal received, shutting down...");
            }
            _ = await_idle_timeout(
                app_state.activity_tracker.clone(),
                options.lifecycle.idle_timeout,
                options.lifecycle.idle_timeout_poll_interval,
            ) => {
                info!("Idle timeout ({:?}) reached", options.lifecycle.idle_timeout);
                info!("Shutting down...");
            }
            _ = await_max_runtime(options.lifecycle.max_runtime) => {
                info!("Max runtime ({:?}) reached, shutting down...", options.lifecycle.max_runtime);
            }
        }
    }
    // if the app is persistent, wait for ctrl-c to trigger a shutdown
    else {
        tokio::select! {
            _ = shutdown_signal => {
                info!("Shutdown signal received, shutting down...");
            }
        }
    }

    // shutdown the server
    drop(shutdown_tx);
    shutdown_manager.shutdown().await
}

async fn await_idle_timeout(
    activity_tracker: Arc<activity::Tracker>,
    idle_timeout: Duration,
    poll_interval: Duration,
) -> Result<(), ServerErr> {
    loop {
        tokio::time::sleep(poll_interval).await;
        let last_activity =
            SystemTime::UNIX_EPOCH + Duration::from_secs(activity_tracker.last_touched());
        match SystemTime::now().duration_since(last_activity) {
            Ok(duration) if duration > idle_timeout => {
                info!("Server idle timeout reached, shutting down...");
                return Ok(());
            }
            Err(_) => {
                error!("Server idle timeout checker error, ignoring...");
            }
            _ => {}
        }
    }
}

async fn await_max_runtime(max_runtime: Duration) -> Result<(), ServerErr> {
    tokio::time::sleep(max_runtime).await;
    Ok(())
}

// =============================== INITIALIZATION ================================== //
async fn init(
    options: &AppOptions,
    shutdown_tx: broadcast::Sender<()>,
    shutdown_manager: &mut ShutdownManager,
) -> Result<Arc<AppState>, ServerErr> {
    let app_state = init_app_state(options, shutdown_manager).await?;

    init_token_refresh_worker(
        app_state.token_mngr.clone(),
        options.token_refresh_worker.clone(),
        shutdown_manager,
        shutdown_tx.subscribe(),
    )
    .await?;

    if options.enable_socket_server {
        init_socket_server(
            options,
            app_state.clone(),
            shutdown_manager,
            shutdown_tx.clone(),
            shutdown_tx.subscribe(),
        )
        .await?;
    }

    if options.enable_poller {
        init_poller_worker(
            options.poller.clone(),
            app_state.clone(),
            shutdown_manager,
            shutdown_tx.subscribe(),
        )
        .await?;
    }

    if options.enable_mqtt_worker {
        init_mqtt_worker(
            options.mqtt_worker.clone(),
            app_state.clone(),
            shutdown_manager,
            shutdown_tx.subscribe(),
        )
        .await?;
    }

    if options.enable_scanner {
        let (scanner, scanner_handle) = init_scanner(options).await?;
        let scanner = Arc::new(scanner);
        shutdown_manager.with_scanner(scanner.clone(), scanner_handle)?;
        init_scan_worker(
            options.scanner.clone(),
            scanner.clone(),
            shutdown_manager,
            shutdown_tx.subscribe(),
        )
        .await?;
        init_scan_bridge_worker(
            scanner.clone(),
            app_state.clone(),
            shutdown_manager,
            shutdown_tx.subscribe(),
        )
        .await?;
    }

    Ok(app_state)
}

async fn init_app_state(
    options: &AppOptions,
    shutdown_manager: &mut ShutdownManager,
) -> Result<Arc<AppState>, ServerErr> {
    let (app_state, app_state_handle) = AppState::init(
        &options.storage.layout,
        options.storage.capacities,
        Arc::new(http::Client::new(&options.backend_host.as_url())?),
        options.dpl_retry_policy,
    )
    .await?;
    let app_state = Arc::new(app_state);
    shutdown_manager.with_app_state(app_state.clone(), Box::pin(app_state_handle))?;

    Ok(app_state)
}

async fn init_token_refresh_worker(
    token_mngr: Arc<authn::TokenManager>,
    options: TokenRefreshWorkerOptions,
    shutdown_manager: &mut ShutdownManager,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> Result<(), ServerErr> {
    info!("Initializing token refresh worker...");

    // refresh the token before starting the refresh worker if it is expired
    if let Err(e) = refresh_if_expired(&token_mngr).await {
        error!("Failed to refresh expired token: {}", e);
    }

    // start the refresh worker
    let token_refresh_handle = tokio::spawn(async move {
        run_token_refresh_worker(
            &options,
            token_mngr.as_ref(),
            |wait| tokio::time::sleep(wait),
            Box::pin(async move {
                let _ = shutdown_rx.recv().await;
            }),
        )
        .await;
    });
    shutdown_manager.register_handle(
        |mgr| &mut mgr.token_refresh_worker_handle,
        "token_refresh_handle",
        token_refresh_handle,
    )?;
    Ok(())
}

async fn refresh_if_expired(token_mngr: &authn::TokenManager) -> Result<(), ServerErr> {
    let token = token_mngr.get_token().await?;
    if token.is_expired() {
        token_mngr.refresh_token().await?;
    }
    Ok(())
}

async fn init_poller_worker(
    options: poller::Options,
    app_state: Arc<AppState>,
    shutdown_manager: &mut ShutdownManager,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> Result<(), ServerErr> {
    info!("Initializing poller worker...");

    let syncer = app_state.syncer.clone();
    let device_stor = app_state.storage.device.clone();

    let poller_handle = tokio::spawn(async move {
        poller::run(
            &options,
            syncer.as_ref(),
            device_stor.as_ref(),
            tokio::time::sleep,
            Box::pin(async move {
                let _ = shutdown_rx.recv().await;
            }),
        )
        .await;
    });
    shutdown_manager.register_handle(
        |mgr| &mut mgr.poller_worker_handle,
        "poller_handle",
        poller_handle,
    )?;
    Ok(())
}

async fn init_mqtt_worker(
    options: mqtt::Options,
    app_state: Arc<AppState>,
    shutdown_manager: &mut ShutdownManager,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> Result<(), ServerErr> {
    info!("Initializing mqtt worker...");

    let token_mngr = app_state.token_mngr.clone();
    let syncer = app_state.syncer.clone();
    let device_stor = app_state.storage.device.clone();

    let mqtt_handle = tokio::spawn(async move {
        mqtt::run(
            &options,
            token_mngr.as_ref(),
            syncer.as_ref(),
            device_stor.as_ref(),
            tokio::time::sleep,
            Box::pin(async move {
                let _ = shutdown_rx.recv().await;
            }),
        )
        .await;
    });
    shutdown_manager.register_handle(
        |mgr| &mut mgr.mqtt_worker_handle,
        "mqtt_handle",
        mqtt_handle,
    )?;
    Ok(())
}

async fn init_scanner(options: &AppOptions) -> Result<(scan::Scanner, JoinHandle<()>), ServerErr> {
    let snapshot_file = ScanSnapshotFile::new_with_default(
        options.storage.layout.scanner_snapshot(),
        Default::default(),
    )
    .await?;
    let (scanner, handle) = scan::Scanner::spawn(
        64,
        ScannerArgs {
            snapshot_file: Some(snapshot_file),
            ..ScannerArgs::default()
        },
    )?;
    Ok((scanner, handle))
}

async fn init_scan_worker(
    options: crate::workers::scan::Options,
    scanner: Arc<scan::Scanner>,
    shutdown_manager: &mut ShutdownManager,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> Result<(), ServerErr> {
    info!("Initializing scan driver worker...");
    let scan_handle = tokio::spawn(async move {
        crate::workers::scan::run(
            &options,
            scanner.as_ref(),
            tokio::time::sleep,
            Box::pin(async move {
                let _ = shutdown_rx.recv().await;
            }),
        )
        .await;
    });
    shutdown_manager.register_handle(
        |mgr| &mut mgr.scan_worker_handle,
        "scan_handle",
        scan_handle,
    )?;
    Ok(())
}

async fn init_scan_bridge_worker(
    scanner: Arc<scan::Scanner>,
    app_state: Arc<AppState>,
    shutdown_manager: &mut ShutdownManager,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> Result<(), ServerErr> {
    info!("Initializing scan bridge worker...");
    let syncer = app_state.syncer.clone();
    let deployments = app_state.storage.deployments.clone();
    let releases = app_state.storage.releases.clone();
    let upload_rules = app_state.storage.upload_rules.clone();
    let bridge_handle = tokio::spawn(async move {
        scan_bridge::run(
            scanner.as_ref(),
            syncer.as_ref(),
            deployments.as_ref(),
            releases.as_ref(),
            upload_rules.as_ref(),
            Box::pin(async move {
                let _ = shutdown_rx.recv().await;
            }),
        )
        .await;
    });
    shutdown_manager.register_handle(
        |mgr| &mut mgr.scan_bridge_worker_handle,
        "scan_bridge_handle",
        bridge_handle,
    )?;
    Ok(())
}

async fn init_socket_server(
    options: &AppOptions,
    app_state: Arc<AppState>,
    shutdown_manager: &mut ShutdownManager,
    shutdown_tx: broadcast::Sender<()>,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> Result<(), ServerErr> {
    info!("Initializing socket server...");

    // run the axum server with graceful shutdown
    let server_state = server::State::new(
        app_state.storage.clone(),
        app_state.http_client.clone(),
        app_state.syncer.clone(),
        app_state.token_mngr.clone(),
        app_state.activity_tracker.clone(),
        app_state.event_hub.clone(),
        shutdown_tx.clone(),
    );
    let server_handle = serve(&options.server, Arc::new(server_state), async move {
        let _ = shutdown_rx.recv().await;
    })
    .await?;
    shutdown_manager.with_socket_server_handle(server_handle)?;

    Ok(())
}

// ================================= SHUTDOWN ===================================== //
struct AppStateShutdownParams {
    state: Arc<AppState>,
    state_handle: Pin<Box<dyn Future<Output = ()> + Send>>,
}

struct ShutdownManager {
    // shutdown transmitter
    shutdown_tx: broadcast::Sender<()>,
    lifecycle_options: LifecycleOptions,

    // server components requiring shutdown
    app_state: Option<AppStateShutdownParams>,
    socket_server_handle: Option<JoinHandle<Result<(), ServerErr>>>,
    poller_worker_handle: Option<JoinHandle<()>>,
    mqtt_worker_handle: Option<JoinHandle<()>>,
    token_refresh_worker_handle: Option<JoinHandle<()>>,
    scanner: Option<(Arc<scan::Scanner>, JoinHandle<()>)>,
    scan_worker_handle: Option<JoinHandle<()>>,
    scan_bridge_worker_handle: Option<JoinHandle<()>>,
}

impl ShutdownManager {
    pub fn new(shutdown_tx: broadcast::Sender<()>, lifecycle_options: LifecycleOptions) -> Self {
        Self {
            shutdown_tx,
            lifecycle_options,
            app_state: None,
            socket_server_handle: None,
            poller_worker_handle: None,
            mqtt_worker_handle: None,
            token_refresh_worker_handle: None,
            scanner: None,
            scan_worker_handle: None,
            scan_bridge_worker_handle: None,
        }
    }

    pub fn with_app_state(
        &mut self,
        state: Arc<AppState>,
        state_handle: Pin<Box<dyn Future<Output = ()> + Send>>,
    ) -> Result<(), ServerErr> {
        if self.app_state.is_some() {
            return Err(ServerErr::ShutdownMngrDuplicateArgErr(
                ShutdownMngrDuplicateArgErr {
                    arg_name: "app_state".to_string(),
                    trace: trace!(),
                },
            ));
        }
        self.app_state = Some(AppStateShutdownParams {
            state,
            state_handle,
        });
        Ok(())
    }

    pub fn with_scanner(
        &mut self,
        scanner: Arc<scan::Scanner>,
        handle: JoinHandle<()>,
    ) -> Result<(), ServerErr> {
        if self.scanner.is_some() {
            return Err(ServerErr::ShutdownMngrDuplicateArgErr(
                ShutdownMngrDuplicateArgErr {
                    arg_name: "scanner".to_string(),
                    trace: trace!(),
                },
            ));
        }
        self.scanner = Some((scanner, handle));
        Ok(())
    }

    pub fn register_handle<Accessor>(
        &mut self,
        slot_for: Accessor,
        handle_name: &str,
        handle: JoinHandle<()>,
    ) -> Result<(), ServerErr>
    where
        Accessor: FnOnce(&mut Self) -> &mut Option<JoinHandle<()>>,
    {
        let slot = slot_for(self);
        if slot.is_some() {
            return Err(ServerErr::ShutdownMngrDuplicateArgErr(
                ShutdownMngrDuplicateArgErr {
                    arg_name: handle_name.to_string(),
                    trace: trace!(),
                },
            ));
        }
        *slot = Some(handle);
        Ok(())
    }

    pub fn with_socket_server_handle(
        &mut self,
        socket_server_handle: JoinHandle<Result<(), ServerErr>>,
    ) -> Result<(), ServerErr> {
        if self.socket_server_handle.is_some() {
            return Err(ServerErr::ShutdownMngrDuplicateArgErr(
                ShutdownMngrDuplicateArgErr {
                    arg_name: "server_handle".to_string(),
                    trace: trace!(),
                },
            ));
        }
        self.socket_server_handle = Some(socket_server_handle);
        Ok(())
    }

    pub async fn shutdown(&mut self) -> Result<(), ServerErr> {
        // send the shutdown signal to all components
        let _ = self.shutdown_tx.send(());

        match tokio::time::timeout(
            self.lifecycle_options.max_shutdown_delay,
            self.shutdown_impl(),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => {
                error!(
                    "Shutdown timed out after {:?}, forcing shutdown...",
                    self.lifecycle_options.max_shutdown_delay
                );
                std::process::exit(1);
            }
        }
    }

    async fn shutdown_impl(&mut self) -> Result<(), ServerErr> {
        // the shutdown order is important here. The refresh and server threads rely on
        // the state so the state must be shutdown last. Shutdown is best-effort: every
        // step is attempted and the first error encountered is returned at the end.
        info!("Shutting down miru agent...");

        let mut first_err: Option<ServerErr> = None;

        // 1. refresh
        if let Some(token_refresh_worker_handle) = self.token_refresh_worker_handle.take() {
            if let Err(e) = token_refresh_worker_handle.await {
                error!("Failed to shutdown token refresh worker: {}", e);
                first_err.get_or_insert_with(|| {
                    ServerErr::JoinHandleErr(JoinHandleErr {
                        source: Box::new(e),
                        trace: trace!(),
                    })
                });
            }
        } else {
            info!(
                "Token refresh worker handle not found, skipping token refresh worker shutdown..."
            );
        }

        // 2. poller
        if let Some(poller_worker_handle) = self.poller_worker_handle.take() {
            if let Err(e) = poller_worker_handle.await {
                error!("Failed to shutdown poller worker: {}", e);
                first_err.get_or_insert_with(|| {
                    ServerErr::JoinHandleErr(JoinHandleErr {
                        source: Box::new(e),
                        trace: trace!(),
                    })
                });
            }
        } else {
            info!("Poller worker handle not found, skipping poller worker shutdown...");
        }

        // 3. mqtt
        if let Some(mqtt_worker_handle) = self.mqtt_worker_handle.take() {
            if let Err(e) = mqtt_worker_handle.await {
                error!("Failed to shutdown MQTT worker: {}", e);
                first_err.get_or_insert_with(|| {
                    ServerErr::JoinHandleErr(JoinHandleErr {
                        source: Box::new(e),
                        trace: trace!(),
                    })
                });
            }
        } else {
            info!("MQTT worker handle not found, skipping MQTT worker shutdown...");
        }

        // 4. server
        if let Some(socket_server_handle) = self.socket_server_handle.take() {
            match socket_server_handle.await {
                Err(e) => {
                    error!("Failed to shutdown socket server: {}", e);
                    first_err.get_or_insert_with(|| {
                        ServerErr::JoinHandleErr(JoinHandleErr {
                            source: Box::new(e),
                            trace: trace!(),
                        })
                    });
                }
                Ok(Err(e)) => {
                    error!("Failed to shutdown socket server: {}", e);
                    first_err.get_or_insert(e);
                }
                Ok(Ok(())) => {}
            }
        } else {
            info!("Socket server handle not found, skipping socket server shutdown...");
        }

        // The scan components must be torn down in driver -> bridge -> actor
        // order. Both the scan driver and scan bridge workers send commands
        // (scan(), update_rules()) INTO the scanner actor. If we closed the
        // actor first, those in-flight commands would fail with
        // SendActorMessageErr. So we join both workers first, then shut down
        // the actor itself.

        // 5. scan driver worker
        if let Some(scan_worker_handle) = self.scan_worker_handle.take() {
            if let Err(e) = scan_worker_handle.await {
                error!("Failed to shutdown scan driver worker: {}", e);
                first_err.get_or_insert_with(|| {
                    ServerErr::JoinHandleErr(JoinHandleErr {
                        source: Box::new(e),
                        trace: trace!(),
                    })
                });
            }
        } else {
            info!("Scan driver worker handle not found, skipping scan driver worker shutdown...");
        }

        // 6. scan bridge worker
        if let Some(scan_bridge_worker_handle) = self.scan_bridge_worker_handle.take() {
            if let Err(e) = scan_bridge_worker_handle.await {
                error!("Failed to shutdown scan bridge worker: {}", e);
                first_err.get_or_insert_with(|| {
                    ServerErr::JoinHandleErr(JoinHandleErr {
                        source: Box::new(e),
                        trace: trace!(),
                    })
                });
            }
        } else {
            info!("Scan bridge worker handle not found, skipping scan bridge worker shutdown...");
        }

        // 7. scanner actor
        if let Some((scanner, handle)) = self.scanner.take() {
            if let Err(e) = scanner.shutdown().await {
                error!("Failed to shutdown scanner actor: {}", e);
                first_err.get_or_insert(e.into());
            }
            if let Err(e) = handle.await {
                error!("Failed to join scanner actor: {}", e);
                first_err.get_or_insert_with(|| {
                    ServerErr::JoinHandleErr(JoinHandleErr {
                        source: Box::new(e),
                        trace: trace!(),
                    })
                });
            }
        } else {
            info!("Scanner not found, skipping scanner actor shutdown...");
        }

        // 8. app state
        if let Some(app_state) = self.app_state.take() {
            if let Err(e) = app_state.state.shutdown().await {
                error!("Failed to shutdown app state: {}", e);
                first_err.get_or_insert(e);
            }
            app_state.state_handle.await;
        } else {
            info!("App state not found, skipping app state shutdown...");
        }

        match first_err {
            Some(e) => Err(e),
            None => {
                info!("Program shutdown complete");
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    // internal crates
    use super::*;
    use crate::deploy::fsm;
    use crate::disk::{Capacities, Layout};
    use crate::filesys::{dirs, files, WriteOptions};
    use crate::models::Device;

    fn new_shutdown_manager() -> ShutdownManager {
        let (shutdown_tx, _) = broadcast::channel(1);
        ShutdownManager::new(shutdown_tx, LifecycleOptions::default())
    }

    fn spawn_immediate_handle() -> tokio::task::JoinHandle<()> {
        tokio::spawn(async {})
    }

    fn spawn_panicking_handle() -> tokio::task::JoinHandle<()> {
        tokio::spawn(async { panic!("boom") })
    }

    fn sentinel_err() -> ServerErr {
        ServerErr::ShutdownMngrDuplicateArgErr(ShutdownMngrDuplicateArgErr {
            arg_name: "sentinel".to_string(),
            trace: trace!(),
        })
    }

    async fn init_app_state() -> (
        dirs::TempDir,
        Arc<AppState>,
        Pin<Box<dyn Future<Output = ()> + Send>>,
    ) {
        let tmp = dirs::temp("shutdown_impl_tests").unwrap();
        let layout = Layout::new(tmp.to_dir());

        let opts = WriteOptions::default();
        files::write_string(&layout.auth().private_key(), "test", opts)
            .await
            .unwrap();
        files::write_string(&layout.auth().public_key(), "test", opts)
            .await
            .unwrap();
        files::write_json(&layout.device(), &Device::default(), opts)
            .await
            .unwrap();

        let (state, state_handle) = AppState::init(
            &layout,
            Capacities::default(),
            Arc::new(http::Client::new("doesntmatter").unwrap()),
            fsm::RetryPolicy::default(),
        )
        .await
        .unwrap();
        (tmp, Arc::new(state), Box::pin(state_handle))
    }

    #[tokio::test]
    async fn register_handle_rejects_token_refresh_duplicates() {
        let mut shutdown_manager = new_shutdown_manager();

        shutdown_manager
            .register_handle(
                |mgr| &mut mgr.token_refresh_worker_handle,
                "token_refresh_handle",
                spawn_immediate_handle(),
            )
            .unwrap();

        let err = shutdown_manager
            .register_handle(
                |mgr| &mut mgr.token_refresh_worker_handle,
                "token_refresh_handle",
                spawn_immediate_handle(),
            )
            .expect_err("duplicate token refresh handle should error");

        match err {
            ServerErr::ShutdownMngrDuplicateArgErr(err) => {
                assert_eq!(err.arg_name, "token_refresh_handle");
            }
            _ => panic!("expected ShutdownMngrDuplicateArgErr"),
        }
    }

    #[tokio::test]
    async fn register_handle_rejects_poller_duplicates() {
        let mut shutdown_manager = new_shutdown_manager();

        shutdown_manager
            .register_handle(
                |mgr| &mut mgr.poller_worker_handle,
                "poller_handle",
                spawn_immediate_handle(),
            )
            .unwrap();

        let err = shutdown_manager
            .register_handle(
                |mgr| &mut mgr.poller_worker_handle,
                "poller_handle",
                spawn_immediate_handle(),
            )
            .expect_err("duplicate poller handle should error");

        match err {
            ServerErr::ShutdownMngrDuplicateArgErr(err) => {
                assert_eq!(err.arg_name, "poller_handle");
            }
            _ => panic!("expected ShutdownMngrDuplicateArgErr"),
        }
    }

    #[tokio::test]
    async fn register_handle_rejects_mqtt_duplicates() {
        let mut shutdown_manager = new_shutdown_manager();

        shutdown_manager
            .register_handle(
                |mgr| &mut mgr.mqtt_worker_handle,
                "mqtt_handle",
                spawn_immediate_handle(),
            )
            .unwrap();

        let err = shutdown_manager
            .register_handle(
                |mgr| &mut mgr.mqtt_worker_handle,
                "mqtt_handle",
                spawn_immediate_handle(),
            )
            .expect_err("duplicate mqtt handle should error");

        match err {
            ServerErr::ShutdownMngrDuplicateArgErr(err) => {
                assert_eq!(err.arg_name, "mqtt_handle");
            }
            _ => panic!("expected ShutdownMngrDuplicateArgErr"),
        }
    }

    // ============================= shutdown_impl ============================= //

    #[tokio::test]
    async fn shutdown_impl_runs_app_state_shutdown_after_worker_panic() {
        let mut mgr = new_shutdown_manager();
        mgr.register_handle(
            |mgr| &mut mgr.token_refresh_worker_handle,
            "token_refresh_handle",
            spawn_panicking_handle(),
        )
        .unwrap();
        mgr.register_handle(
            |mgr| &mut mgr.poller_worker_handle,
            "poller_handle",
            spawn_immediate_handle(),
        )
        .unwrap();
        mgr.register_handle(
            |mgr| &mut mgr.mqtt_worker_handle,
            "mqtt_handle",
            spawn_immediate_handle(),
        )
        .unwrap();
        mgr.with_socket_server_handle(tokio::spawn(async { Ok(()) }))
            .unwrap();
        let (_tmp, state, state_handle) = init_app_state().await;
        mgr.with_app_state(state, state_handle).unwrap();

        let err = mgr
            .shutdown_impl()
            .await
            .expect_err("worker panic should surface");

        // every slot drained proves all five steps were attempted despite the
        // token refresh worker panicking in step 1
        assert!(matches!(err, ServerErr::JoinHandleErr(_)));
        assert!(mgr.token_refresh_worker_handle.is_none());
        assert!(mgr.poller_worker_handle.is_none());
        assert!(mgr.mqtt_worker_handle.is_none());
        assert!(mgr.socket_server_handle.is_none());
        assert!(mgr.app_state.is_none());
    }

    #[tokio::test]
    async fn shutdown_impl_returns_first_error() {
        let mut mgr = new_shutdown_manager();
        mgr.register_handle(
            |mgr| &mut mgr.poller_worker_handle,
            "poller_handle",
            spawn_panicking_handle(),
        )
        .unwrap();
        mgr.register_handle(
            |mgr| &mut mgr.mqtt_worker_handle,
            "mqtt_handle",
            spawn_panicking_handle(),
        )
        .unwrap();
        mgr.with_socket_server_handle(tokio::spawn(async { Err(sentinel_err()) }))
            .unwrap();

        let err = mgr
            .shutdown_impl()
            .await
            .expect_err("poller panic should surface");

        // the poller's join error (step 2) wins over the mqtt join error
        // (step 3) and the socket server's sentinel error (step 4)
        assert!(matches!(err, ServerErr::JoinHandleErr(_)));
        assert!(mgr.socket_server_handle.is_none());
    }

    #[tokio::test]
    async fn shutdown_impl_returns_socket_server_inner_error() {
        let mut mgr = new_shutdown_manager();
        mgr.with_socket_server_handle(tokio::spawn(async { Err(sentinel_err()) }))
            .unwrap();

        let err = mgr
            .shutdown_impl()
            .await
            .expect_err("socket server error should surface");

        match err {
            ServerErr::ShutdownMngrDuplicateArgErr(err) => {
                assert_eq!(err.arg_name, "sentinel");
            }
            _ => panic!("expected ShutdownMngrDuplicateArgErr"),
        }
    }

    #[tokio::test]
    async fn shutdown_impl_maps_socket_server_join_error() {
        let mut mgr = new_shutdown_manager();
        mgr.with_socket_server_handle(tokio::spawn(async { panic!("boom") }))
            .unwrap();

        let err = mgr
            .shutdown_impl()
            .await
            .expect_err("socket server panic should surface");

        assert!(matches!(err, ServerErr::JoinHandleErr(_)));
        assert!(mgr.socket_server_handle.is_none());
    }

    #[tokio::test]
    async fn shutdown_impl_ok_when_all_steps_succeed() {
        let mut mgr = new_shutdown_manager();
        mgr.register_handle(
            |mgr| &mut mgr.token_refresh_worker_handle,
            "token_refresh_handle",
            spawn_immediate_handle(),
        )
        .unwrap();
        mgr.register_handle(
            |mgr| &mut mgr.poller_worker_handle,
            "poller_handle",
            spawn_immediate_handle(),
        )
        .unwrap();
        mgr.register_handle(
            |mgr| &mut mgr.mqtt_worker_handle,
            "mqtt_handle",
            spawn_immediate_handle(),
        )
        .unwrap();
        mgr.with_socket_server_handle(tokio::spawn(async { Ok(()) }))
            .unwrap();

        mgr.shutdown_impl().await.unwrap();

        assert!(mgr.token_refresh_worker_handle.is_none());
        assert!(mgr.poller_worker_handle.is_none());
        assert!(mgr.mqtt_worker_handle.is_none());
        assert!(mgr.socket_server_handle.is_none());
    }

    // ============================= scanner slots ============================= //

    fn spawn_scanner() -> (Arc<scan::Scanner>, JoinHandle<()>) {
        let (scanner, handle) = scan::Scanner::spawn(64, ScannerArgs::default()).unwrap();
        (Arc::new(scanner), handle)
    }

    #[tokio::test]
    async fn register_handle_rejects_scan_worker_duplicates() {
        let mut shutdown_manager = new_shutdown_manager();

        shutdown_manager
            .register_handle(
                |mgr| &mut mgr.scan_worker_handle,
                "scan_handle",
                spawn_immediate_handle(),
            )
            .unwrap();

        let err = shutdown_manager
            .register_handle(
                |mgr| &mut mgr.scan_worker_handle,
                "scan_handle",
                spawn_immediate_handle(),
            )
            .expect_err("duplicate scan handle should error");

        match err {
            ServerErr::ShutdownMngrDuplicateArgErr(err) => {
                assert_eq!(err.arg_name, "scan_handle");
            }
            _ => panic!("expected ShutdownMngrDuplicateArgErr"),
        }
    }

    #[tokio::test]
    async fn register_handle_rejects_scan_bridge_duplicates() {
        let mut shutdown_manager = new_shutdown_manager();

        shutdown_manager
            .register_handle(
                |mgr| &mut mgr.scan_bridge_worker_handle,
                "scan_bridge_handle",
                spawn_immediate_handle(),
            )
            .unwrap();

        let err = shutdown_manager
            .register_handle(
                |mgr| &mut mgr.scan_bridge_worker_handle,
                "scan_bridge_handle",
                spawn_immediate_handle(),
            )
            .expect_err("duplicate scan bridge handle should error");

        match err {
            ServerErr::ShutdownMngrDuplicateArgErr(err) => {
                assert_eq!(err.arg_name, "scan_bridge_handle");
            }
            _ => panic!("expected ShutdownMngrDuplicateArgErr"),
        }
    }

    #[tokio::test]
    async fn with_scanner_rejects_duplicates() {
        let mut shutdown_manager = new_shutdown_manager();

        let (scanner, handle) = spawn_scanner();
        shutdown_manager.with_scanner(scanner, handle).unwrap();

        let (scanner, handle) = spawn_scanner();
        let err = shutdown_manager
            .with_scanner(scanner, handle)
            .expect_err("duplicate scanner should error");

        match err {
            ServerErr::ShutdownMngrDuplicateArgErr(err) => {
                assert_eq!(err.arg_name, "scanner");
            }
            _ => panic!("expected ShutdownMngrDuplicateArgErr"),
        }
    }

    #[tokio::test]
    async fn shutdown_impl_ok_when_scan_steps_succeed() {
        let mut mgr = new_shutdown_manager();
        mgr.register_handle(
            |mgr| &mut mgr.scan_worker_handle,
            "scan_handle",
            spawn_immediate_handle(),
        )
        .unwrap();
        mgr.register_handle(
            |mgr| &mut mgr.scan_bridge_worker_handle,
            "scan_bridge_handle",
            spawn_immediate_handle(),
        )
        .unwrap();
        let (scanner, handle) = spawn_scanner();
        mgr.with_scanner(scanner, handle).unwrap();

        mgr.shutdown_impl().await.unwrap();

        // every scan slot drained proves the driver -> bridge -> actor
        // teardown ran to completion
        assert!(mgr.scan_worker_handle.is_none());
        assert!(mgr.scan_bridge_worker_handle.is_none());
        assert!(mgr.scanner.is_none());
    }
}
