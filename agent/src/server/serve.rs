// standard crates
use std::future::Future;
use std::sync::Arc;
use std::{
    env,
    os::unix::io::{FromRawFd, RawFd},
};

// internal crates
use super::errors::{BindUnixSocketErr, RunAxumServerErr, ServerErr};
use super::handlers;
use super::state::State;
use crate::filesys;
use crate::filesys::PathExt;
use crate::trace;

// external crates
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
pub struct Options {
    pub socket_file: filesys::File,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            socket_file: filesys::File::new("/run/miru/miru.sock"),
        }
    }
}

const API_VERSION: &str = "v0.2";

/// Build the application router with all routes and shared state, without middleware.
pub fn routes(state: Arc<State>) -> Router {
    Router::new()
        // =============================== AGENT INFO ============================== //
        .route(
            format!("/{API_VERSION}/health").as_str(),
            get(handlers::health),
        )
        .route(
            format!("/{API_VERSION}/version").as_str(),
            get(handlers::version),
        )
        // ============================= DEVICE ==================================== //
        .route(
            format!("/{API_VERSION}/device").as_str(),
            get(handlers::get_device),
        )
        .route(
            format!("/{API_VERSION}/device/sync").as_str(),
            post(handlers::sync_device),
        )
        // ============================= DEPLOYMENTS =============================== //
        // /current before /{id} so "current" isn't captured as a deployment_id
        .route(
            format!("/{API_VERSION}/deployments/current").as_str(),
            get(handlers::get_current_deployment),
        )
        .route(
            format!("/{API_VERSION}/deployments/{{deployment_id}}").as_str(),
            get(handlers::get_deployment),
        )
        // ============================= RELEASES ================================== //
        // /current before /{id} so "current" isn't captured as a release_id
        .route(
            format!("/{API_VERSION}/releases/current").as_str(),
            get(handlers::get_current_release),
        )
        .route(
            format!("/{API_VERSION}/releases/{{release_id}}").as_str(),
            get(handlers::get_release),
        )
        // ============================= GIT COMMITS =============================== //
        .route(
            format!("/{API_VERSION}/git_commits/{{git_commit_id}}").as_str(),
            get(handlers::get_git_commit),
        )
        .with_state(state)
}

pub(crate) async fn serve(
    options: &Options,
    state: Arc<State>,
    shutdown_signal: impl Future<Output = ()> + Send + 'static,
) -> Result<JoinHandle<Result<(), ServerErr>>, ServerErr> {
    let state_for_middleware = state.clone();
    let app = routes(state)
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
        );

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
    socket_file: &filesys::File,
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

async fn create_unix_socket_listener(
    socket_file: &filesys::File,
) -> Result<UnixListener, ServerErr> {
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
