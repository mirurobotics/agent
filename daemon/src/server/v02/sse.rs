// standard crates
use std::sync::Arc;

// internal crates
use crate::errors::Error;
use crate::events::{errors::EventsErr, model::Event};
use crate::server::{
    sse::{self, EventsQuery},
    state::State,
};
use device_api_v02::models as device_server;

// external crates
use axum::{
    extract::{Query, State as AxumState},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use tracing::error;

pub async fn events(
    AxumState(state): AxumState<Arc<State>>,
    Query(params): Query<EventsQuery>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    sse::event_stream(state, params, headers, to_sse_data)
        .await
        .map_err(|e| {
            error!("SSE error: {e:?}");
            let err_response = device_server::ErrorResponse {
                error: Box::new(device_server::Error {
                    code: e.code().as_str().to_string(),
                    params: Default::default(),
                    message: e.to_string(),
                }),
            };
            (e.http_status(), Json(serde_json::json!(err_response)))
        })
}

fn to_sse_data(event: &Event) -> Result<String, EventsErr> {
    let api_event = device_server::Event::try_from(event)?;
    Ok(serde_json::to_string(&api_event)?)
}
