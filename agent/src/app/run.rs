// standard library
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
use crate::authn::token_mngr::{TokenManager, TokenManagerExt};
use crate::http;
use crate::server::{errors::*, serve::serve, state::ServerState};
use crate::trace;
use crate::workers::{
    mqtt, poller,
    token_refresh::{run_token_refresh_worker, TokenRefreshWorkerOptions},
};

// external
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::{error, info};

pub async fn run(
    agent_version: String,
    options: AppOptions,
    shutdown_signal: impl Future<Output = ()> + Send + 'static,
) -> Result<(), ServerErr> {
    info!("Initializing miru agent...");

    // Create a single shutdown channel that all components will listen to
    let (shutdown_tx, _shutdown_rx): (tokio::sync::broadcast::Sender<()>, _) =
        tokio::sync::broadcast::channel(1);
    let mut shutdown_manager = ShutdownManager::new(shutdown_tx.clone(), options.lifecycle);

    // initialize the app (and shutdown if failures occur)
    let app_state = match init(
        agent_version,
        &options,
        shutdown_tx.clone(),
        &mut shutdown_manager,
    )
    .await
    {
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
    agent_version: String,
    options: &AppOptions,
    shutdown_tx: broadcast::Sender<()>,
    shutdown_manager: &mut ShutdownManager,
) -> Result<Arc<AppState>, ServerErr> {
    let app_state = init_app_state(agent_version, options, shutdown_manager).await?;

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

    Ok(app_state)
}

async fn init_app_state(
    agent_version: String,
    options: &AppOptions,
    shutdown_manager: &mut ShutdownManager,
) -> Result<Arc<AppState>, ServerErr> {
    let (app_state, app_state_handle) = AppState::init(
        agent_version,
        &options.storage.layout,
        options.storage.cache_capacities,
        Arc::new(http::Client::new(&options.backend_base_url).await),
        options.dpl_retry_policy,
    )
    .await?;
    let app_state = Arc::new(app_state);
    shutdown_manager.with_app_state(app_state.clone(), Box::pin(app_state_handle))?;

    Ok(app_state)
}

async fn init_token_refresh_worker(
    token_mngr: Arc<TokenManager>,
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
    shutdown_manager.with_token_refresh_worker_handle(token_refresh_handle)?;
    Ok(())
}

async fn refresh_if_expired(token_mngr: &TokenManager) -> Result<(), ServerErr> {
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
    let device_file = app_state.device_file.clone();

    let poller_handle = tokio::spawn(async move {
        poller::run(
            &options,
            syncer.as_ref(),
            device_file.as_ref(),
            tokio::time::sleep,
            Box::pin(async move {
                let _ = shutdown_rx.recv().await;
            }),
        )
        .await;
    });
    shutdown_manager.with_poller_worker_handle(poller_handle)?;
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
    let device_file = app_state.device_file.clone();

    let mqtt_handle = tokio::spawn(async move {
        mqtt::run(
            &options,
            token_mngr.as_ref(),
            syncer.as_ref(),
            device_file.as_ref(),
            tokio::time::sleep,
            Box::pin(async move {
                let _ = shutdown_rx.recv().await;
            }),
        )
        .await;
    });
    shutdown_manager.with_mqtt_worker_handle(mqtt_handle)?;
    Ok(())
}

async fn init_socket_server(
    options: &AppOptions,
    app_state: Arc<AppState>,
    shutdown_manager: &mut ShutdownManager,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> Result<(), ServerErr> {
    info!("Initializing socket server...");

    // run the axum server with graceful shutdown
    let server_state = ServerState::new(
        app_state.device_file.clone(),
        app_state.http_client.clone(),
        app_state.syncer.clone(),
        app_state.caches.clone(),
        app_state.token_mngr.clone(),
        app_state.activity_tracker.clone(),
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

    pub fn with_token_refresh_worker_handle(
        &mut self,
        token_refresh_handle: JoinHandle<()>,
    ) -> Result<(), ServerErr> {
        if self.token_refresh_worker_handle.is_some() {
            return Err(ServerErr::ShutdownMngrDuplicateArgErr(
                ShutdownMngrDuplicateArgErr {
                    arg_name: "token_refresh_handle".to_string(),
                    trace: trace!(),
                },
            ));
        }
        self.token_refresh_worker_handle = Some(token_refresh_handle);
        Ok(())
    }

    pub fn with_poller_worker_handle(
        &mut self,
        poller_handle: JoinHandle<()>,
    ) -> Result<(), ServerErr> {
        if self.poller_worker_handle.is_some() {
            return Err(ServerErr::ShutdownMngrDuplicateArgErr(
                ShutdownMngrDuplicateArgErr {
                    arg_name: "poller_handle".to_string(),
                    trace: trace!(),
                },
            ));
        }
        self.poller_worker_handle = Some(poller_handle);
        Ok(())
    }

    pub fn with_mqtt_worker_handle(
        &mut self,
        mqtt_handle: JoinHandle<()>,
    ) -> Result<(), ServerErr> {
        if self.mqtt_worker_handle.is_some() {
            return Err(ServerErr::ShutdownMngrDuplicateArgErr(
                ShutdownMngrDuplicateArgErr {
                    arg_name: "mqtt_handle".to_string(),
                    trace: trace!(),
                },
            ));
        }
        self.mqtt_worker_handle = Some(mqtt_handle);
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
        // the state so the state must be shutdown last.
        info!("Shutting down miru agent...");

        // 1. refresh
        if let Some(token_refresh_worker_handle) = self.token_refresh_worker_handle.take() {
            token_refresh_worker_handle.await.map_err(|e| {
                ServerErr::JoinHandleErr(JoinHandleErr {
                    source: Box::new(e),
                    trace: trace!(),
                })
            })?;
        } else {
            info!(
                "Token refresh worker handle not found, skipping token refresh worker shutdown..."
            );
        }

        // 2. poller
        if let Some(poller_worker_handle) = self.poller_worker_handle.take() {
            poller_worker_handle.await.map_err(|e| {
                ServerErr::JoinHandleErr(JoinHandleErr {
                    source: Box::new(e),
                    trace: trace!(),
                })
            })?;
        } else {
            info!("Poller worker handle not found, skipping poller worker shutdown...");
        }

        // 3. mqtt
        if let Some(mqtt_worker_handle) = self.mqtt_worker_handle.take() {
            mqtt_worker_handle.await.map_err(|e| {
                ServerErr::JoinHandleErr(JoinHandleErr {
                    source: Box::new(e),
                    trace: trace!(),
                })
            })?;
        } else {
            info!("MQTT worker handle not found, skipping MQTT worker shutdown...");
        }

        // 4. server
        if let Some(socket_server_handle) = self.socket_server_handle.take() {
            socket_server_handle.await.map_err(|e| {
                ServerErr::JoinHandleErr(JoinHandleErr {
                    source: Box::new(e),
                    trace: trace!(),
                })
            })??;
        } else {
            info!("Socket server handle not found, skipping socket server shutdown...");
        }

        // 5. app state
        if let Some(app_state) = self.app_state.take() {
            app_state.state.shutdown().await?;
            app_state.state_handle.await;
        } else {
            info!("App state not found, skipping app state shutdown...");
        }

        info!("Program shutdown complete");
        Ok(())
    }
}
