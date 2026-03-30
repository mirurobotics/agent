// standard crates
use std::collections::HashSet;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::errors::Error;
use crate::events::errors::{EventsErr, MalformedCursorErr};
use crate::server::state::State;
use crate::services::{events as events_svc, ServiceErr};
use crate::trace;
use device_api::models as device_server;

// external crates
use axum::{
    extract::{Query, State as AxumState},
    http::{HeaderMap, StatusCode},
    response::{
        sse::{Event as SseEvent, KeepAlive, Sse},
        IntoResponse,
    },
    Json,
};
use serde::Deserialize;
use tokio_stream::StreamExt;
use tracing::error;

#[derive(Deserialize)]
pub struct EventsQuery {
    pub after: Option<String>,
    pub types: Option<String>,
}

pub async fn events(
    AxumState(state): AxumState<Arc<State>>,
    Query(params): Query<EventsQuery>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    events_impl(state, params, headers).await.map_err(|e| {
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

async fn events_impl(
    state: Arc<State>,
    params: EventsQuery,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ServiceErr> {
    let cursor = resolve_cursor(&params, &headers)?;
    let filter = parse_event_filter(params.types);

    let stream = events_svc::subscribe(&state.event_hub, cursor, filter).await?;

    let sse_stream = stream.filter_map(|event| {
        let json = serde_json::to_string(&event).ok()?;
        Some(Ok::<_, Infallible>(
            SseEvent::default()
                .id(event.id.to_string())
                .event(event.event_type.clone())
                .data(json),
        ))
    });

    Ok(Sse::new(sse_stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(30))
            .text("heartbeat"),
    ))
}

fn resolve_cursor(params: &EventsQuery, headers: &HeaderMap) -> Result<Option<u64>, EventsErr> {
    if let Some(after) = &params.after {
        return after
            .parse::<u64>()
            .map(Some)
            .map_err(|_| EventsErr::MalformedCursorErr(MalformedCursorErr { trace: trace!() }));
    }
    if let Some(last_event_id) = headers.get("Last-Event-ID") {
        let s = last_event_id
            .to_str()
            .map_err(|_| EventsErr::MalformedCursorErr(MalformedCursorErr { trace: trace!() }))?;
        return s
            .parse::<u64>()
            .map(Some)
            .map_err(|_| EventsErr::MalformedCursorErr(MalformedCursorErr { trace: trace!() }));
    }
    Ok(None)
}

fn parse_event_filter(types: Option<String>) -> Option<HashSet<String>> {
    types.map(|t| {
        t.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    })
}
