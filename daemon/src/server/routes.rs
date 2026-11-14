// standard crates
use std::sync::Arc;

// internal crates
use crate::server::{state::State, v02};

// external crates
use axum::{
    extract::{DefaultBodyLimit, Request},
    http::header::{Entry, AUTHORIZATION},
    middleware::{from_fn, Next},
    response::Response,
    Router,
};
use tower::ServiceBuilder;
use tower_http::{
    trace::{DefaultMakeSpan, DefaultOnRequest, DefaultOnResponse, TraceLayer},
    LatencyUnit,
};
use tracing::Level;

/// The largest request body any route accepts, sized for a calibration: 1 MiB
/// of content doubled by JSON escaping (newlines, quotes), plus room for the
/// other fields.
pub const MAX_REQUEST_BYTES: usize = 2 * 1024 * 1024 + 64 * 1024;

/// Build the router both transports serve, including shared middleware.
pub fn router(state: Arc<State>) -> Router {
    middleware(v02::routes::table().with_state(state))
}

/// Authorization redaction, request tracing and the request body limit
/// applied to every transport. Redaction runs outermost so trace spans never
/// log a token.
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
            )
            .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES)),
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
