// standard crates
use std::sync::Arc;

// internal crates
use crate::server::{handlers, state::State};

// external crates
use axum::{
    extract::Request,
    http::header::{Entry, AUTHORIZATION},
    middleware::{from_fn, Next},
    response::Response,
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
    middleware(table(state))
}

/// Authorization redaction and request tracing applied to every transport.
/// Redaction runs outermost so trace spans never log a token.
fn middleware(router: Router) -> Router {
    router.layer(
        ServiceBuilder::new()
            .layer(from_fn(redact_authorization))
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

/// Mark every Authorization header value sensitive so Debug output, including
/// `TraceLayer` spans, prints `Sensitive` instead of the credential.
async fn redact_authorization(mut req: Request, next: Next) -> Response {
    if let Entry::Occupied(mut entry) = req.headers_mut().entry(AUTHORIZATION) {
        for value in entry.iter_mut() {
            value.set_sensitive(true);
        }
    }
    next.run(req).await
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
        // ============================= FILE RULES ================================ //
        .route(
            format!("/{api_version}/file_rules/{{file_rule_id}}").as_str(),
            get(handlers::get_file_rule),
        )
        // ============================== EVENTS =================================== //
        .route(
            format!("/{api_version}/events").as_str(),
            get(super::sse::events),
        )
        .with_state(state)
}
