// standard library
use std::future::Future;
use std::sync::Arc;
use std::{
    env,
    os::unix::io::{FromRawFd, RawFd},
};

// internal crates
use crate::filesys::{file::File, path::PathExt};
use crate::server::errors::{BindUnixSocketErr, RunAxumServerErr, ServerErr};
use crate::server::handlers;
use crate::server::state::ServerState;
use crate::trace;

// external
use axum::{
    routing::{get, post},
    Router,
};
use tokio::net::UnixListener;
use tokio::task::JoinHandle;
use tower::ServiceBuilder;
use tower_http::{
    trace::{DefaultMakeSpan, DefaultOnRequest, DefaultOnResponse, TraceLayer},
    LatencyUnit,
};
use tracing::Level;

#[derive(Debug)]
pub struct ServerOptions {
    pub socket_file: File,
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            socket_file: File::new("/run/miru/miru.sock"),
        }
    }
}

pub(crate) async fn serve(
    options: &ServerOptions,
    state: Arc<ServerState>,
    shutdown_signal: impl Future<Output = ()> + Send + 'static,
) -> Result<JoinHandle<Result<(), ServerErr>>, ServerErr> {
    // build the app with the test route
    let state_for_middleware = state.clone();
    let app = Router::new()
        // =============================== AGENT INFO ============================== //
        .route("/v1/health", get(handlers::health))
        .route("/v1/version", get(handlers::version))
        // ============================= DEVICE ==================================== //
        .route("/v1/device", get(handlers::get_device))
        .route("/v1/device/sync", post(handlers::sync_device))
        // ============================= LAYERS ===================================== //
        .layer(
            ServiceBuilder::new()
                // activity middleware
                .layer(axum::middleware::from_fn(
                    move |req: axum::extract::Request, next: axum::middleware::Next| {
                        let state = state_for_middleware.clone();
                        async move {
                            state.activity_tracker.touch();
                            next.run(req).await
                        }
                    },
                ))
                // logging middleware
                .layer(
                    TraceLayer::new_for_http()
                        .make_span_with(DefaultMakeSpan::new().include_headers(true))
                        .on_request(DefaultOnRequest::new().level(Level::INFO))
                        .on_response(
                            DefaultOnResponse::new()
                                .level(Level::INFO)
                                .latency_unit(LatencyUnit::Micros),
                        ),
                ),
        )
        .with_state(state);

    // obtain the unix socket file listener
    let listener = acquire_unix_socket_listener(&options.socket_file, async move {
        create_unix_socket_listener(&options.socket_file).await
    })
    .await?;

    // serve with graceful shutdown
    let server_handle = tokio::task::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal)
            .await
            .map_err(|e| {
                ServerErr::RunAxumServerErr(RunAxumServerErr {
                    source: e,
                    trace: trace!(),
                })
            })
    });

    Ok(server_handle)
}

async fn acquire_unix_socket_listener(
    socket_file: &File,
    fallback: impl Future<Output = Result<UnixListener, ServerErr>>,
) -> Result<UnixListener, ServerErr> {
    let listener = if let Ok(listen_fds) = env::var("LISTEN_FDS") {
        let listen_fds = listen_fds.parse::<u32>().map_err(|e| {
            ServerErr::BindUnixSocketErr(BindUnixSocketErr {
                socket_file: socket_file.clone(),
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("Failed to parse LISTEN_FDS: {e}"),
                ),
                trace: trace!(),
            })
        })?;
        if listen_fds >= 1 {
            // FD#3 is the first one
            let fd: RawFd = 3;
            // SAFETY: fd=3 was handed to us by systemd
            let std_listener = unsafe { std::os::unix::net::UnixListener::from_raw_fd(fd) };
            std_listener.set_nonblocking(true).map_err(|e| {
                ServerErr::BindUnixSocketErr(BindUnixSocketErr {
                    socket_file: socket_file.clone(),
                    source: e,
                    trace: trace!(),
                })
            })?;
            UnixListener::from_std(std_listener).map_err(|e| {
                ServerErr::BindUnixSocketErr(BindUnixSocketErr {
                    socket_file: socket_file.clone(),
                    source: e,
                    trace: trace!(),
                })
            })?
        } else {
            fallback.await?
        }
    } else {
        fallback.await?
    };
    Ok(listener)
}

async fn create_unix_socket_listener(socket_file: &File) -> Result<UnixListener, ServerErr> {
    socket_file.delete().await?;
    let socket_path = socket_file.path();
    tokio::net::UnixListener::bind(socket_path).map_err(|e| {
        ServerErr::BindUnixSocketErr(BindUnixSocketErr {
            socket_file: socket_file.clone(),
            source: e,
            trace: trace!(),
        })
    })
}
