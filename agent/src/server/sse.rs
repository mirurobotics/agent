// standard crates
use std::collections::HashSet;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::errors::Error;
use crate::events::{
    errors::{EventsErr, MalformedCursorErr},
    model::Envelope,
};
use crate::server::state::State;
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
use tokio_stream::wrappers::BroadcastStream;
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
    // resolve cursor: after query param > Last-Event-ID header > 0
    let cursor = resolve_cursor(&params, &headers).map_err(error_response)?;

    // parse type filter
    let type_filter: Option<HashSet<String>> = params.types.map(|t| {
        t.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    });

    let event_hub = match &state.event_hub {
        Some(hub) => hub,
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "error": {
                        "code": "service_unavailable",
                        "message": "event hub not initialized",
                        "params": {}
                    }
                })),
            ));
        }
    };

    // subscribe BEFORE replay to prevent gaps
    let broadcast_rx = event_hub.subscribe();

    // replay historical events
    let replayed: Vec<Envelope> = event_hub
        .replay_after(cursor)
        .await
        .map_err(error_response)?;

    let last_replayed_id = replayed.last().map(|e| e.id).unwrap_or(cursor);

    // filter replayed events
    let replayed: Vec<Envelope> = match &type_filter {
        Some(filter) => replayed
            .into_iter()
            .filter(|e| filter.contains(&e.event_type))
            .collect(),
        None => replayed,
    };

    // build SSE stream: replay then live
    let replay_stream = tokio_stream::iter(replayed.into_iter().map(Ok::<Envelope, Infallible>));

    let live_stream = BroadcastStream::new(broadcast_rx)
        .filter_map(|result: Result<Envelope, _>| result.ok())
        .filter(move |envelope: &Envelope| envelope.id > last_replayed_id);

    let type_filter_live = type_filter;
    let combined = replay_stream
        .chain(live_stream.map(Ok::<Envelope, Infallible>))
        .filter_map(move |result: Result<Envelope, Infallible>| {
            let envelope = result.ok()?;
            if let Some(ref filter) = type_filter_live {
                if !filter.contains(&envelope.event_type) {
                    return None;
                }
            }
            let json = serde_json::to_string(&envelope).ok()?;
            Some(Ok::<_, Infallible>(
                SseEvent::default()
                    .id(envelope.id.to_string())
                    .event(envelope.event_type.clone())
                    .data(json),
            ))
        });

    Ok(Sse::new(combined).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(30))
            .text("heartbeat"),
    ))
}

fn resolve_cursor(params: &EventsQuery, headers: &HeaderMap) -> Result<u64, EventsErr> {
    if let Some(after) = &params.after {
        return after
            .parse::<u64>()
            .map_err(|_| EventsErr::MalformedCursorErr(MalformedCursorErr { trace: trace!() }));
    }
    if let Some(last_event_id) = headers.get("Last-Event-ID") {
        let s = last_event_id
            .to_str()
            .map_err(|_| EventsErr::MalformedCursorErr(MalformedCursorErr { trace: trace!() }))?;
        return s
            .parse::<u64>()
            .map_err(|_| EventsErr::MalformedCursorErr(MalformedCursorErr { trace: trace!() }));
    }
    Ok(0)
}

fn error_response(e: EventsErr) -> (StatusCode, Json<serde_json::Value>) {
    let status = e.http_status();
    error!("SSE error: {e:?}");
    let err_response = device_server::ErrorResponse {
        error: Box::new(device_server::Error {
            code: "events_error".to_string(),
            params: Default::default(),
            message: e.to_string(),
        }),
    };
    (status, Json(serde_json::json!(err_response)))
}
