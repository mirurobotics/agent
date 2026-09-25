// standard crates
use std::sync::Arc;

// internal crates
use crate::filesys;
use crate::server::{handlers, state::State};

// external crates
use axum::{
    extract::Request,
    middleware::{self, Next},
    routing::{get, post},
    Router,
};
use tower::ServiceBuilder;
use tower_http::{
    trace::{DefaultMakeSpan, DefaultOnRequest, DefaultOnResponse, TraceLayer},
    LatencyUnit,
};
use tracing::Level;

#[derive(Debug)]
pub struct Options {
    /// Unix socket path (unix only).
    pub socket_file: filesys::File,
    /// Loopback TCP port; `None` disables the TCP transport, `Some(0)` lets
    /// the OS assign a free port.
    pub tcp_port: Option<u16>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            socket_file: filesys::File::new("/run/miru/miru.sock"),
            tcp_port: None,
        }
    }
}

/// Build the application router with all routes, shared state, and middleware.
/// Every transport serves this same app.
pub fn app(state: Arc<State>) -> Router {
    let activity_state = state.clone();
    routes(state).layer(
        ServiceBuilder::new()
            .layer(middleware::from_fn(move |req: Request, next: Next| {
                let state = activity_state.clone();
                async move {
                    state.activity_tracker.touch();
                    next.run(req).await
                }
            }))
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
}

/// Build the application router with all routes and shared state, without middleware.
pub fn routes(state: Arc<State>) -> Router {
    let api_version = device_api::models::ApiVersion::API_VERSION.to_string();
    Router::new()
        // =============================== AGENT INFO ============================== //
        .route(
            format!("/{api_version}/health").as_str(),
            get(handlers::health),
        )
        .route(
            format!("/{api_version}/version").as_str(),
            get(handlers::version),
        )
        // ============================= DEVICE ==================================== //
        .route(
            format!("/{api_version}/device").as_str(),
            get(handlers::get_device),
        )
        .route(
            format!("/{api_version}/device/sync").as_str(),
            post(handlers::sync_device),
        )
        // ============================= DEPLOYMENTS =============================== //
        // /current before /{id} so "current" isn't captured as a deployment_id
        .route(
            format!("/{api_version}/deployments/current").as_str(),
            get(handlers::get_current_deployment),
        )
        .route(
            format!("/{api_version}/deployments/{{deployment_id}}").as_str(),
            get(handlers::get_deployment),
        )
        // ============================= RELEASES ================================== //
        // /current before /{id} so "current" isn't captured as a release_id
        .route(
            format!("/{api_version}/releases/current").as_str(),
            get(handlers::get_current_release),
        )
        .route(
            format!("/{api_version}/releases/{{release_id}}").as_str(),
            get(handlers::get_release),
        )
        // ============================= GIT COMMITS =============================== //
        .route(
            format!("/{api_version}/git_commits/{{git_commit_id}}").as_str(),
            get(handlers::get_git_commit),
        )
        // ============================== EVENTS =================================== //
        .route(
            format!("/{api_version}/events").as_str(),
            get(super::sse::events),
        )
        .with_state(state)
}
