// standard crates
use std::sync::Arc;

// internal crates
use crate::server::{handlers, state::State};

// external crates
use axum::{
    extract::Request,
    middleware::{from_fn, Next},
    routing::{get, post},
    Router,
};
use tower::ServiceBuilder;
use tower_http::{
    trace::{DefaultMakeSpan, DefaultOnRequest, DefaultOnResponse, TraceLayer},
    LatencyUnit,
};
use tracing::Level;

/// Build the router both transports serve, including shared middleware.
pub fn router(state: Arc<State>) -> Router {
    middleware(table(state.clone()), state)
}

/// Activity tracking and request tracing applied to every transport.
fn middleware(router: Router, state: Arc<State>) -> Router {
    router.layer(
        ServiceBuilder::new()
            .layer(from_fn(move |req: Request, next: Next| {
                let state = state.clone();
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

fn table(state: Arc<State>) -> Router {
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
