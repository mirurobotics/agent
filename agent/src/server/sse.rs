// standard crates
use std::collections::HashSet;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::errors::Error;
use crate::events::{
    hub::EventHub,
    errors::{EventsErr, MalformedCursorErr},
    model::Event,
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
use futures::Stream;
use serde::Deserialize;
use tokio::sync::broadcast;
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
    let cursor = resolve_cursor(&params, &headers).map_err(error_response)?;
    let filter = parse_event_filter(params.types);
    let event_hub = &state.event_hub;

    // subscribe BEFORE replay to prevent gaps
    let broadcast_rx = event_hub.subscribe();

    let replays = get_replay_events(event_hub, cursor).await.map_err(error_response)?;

    let stream = new_event_stream(replays, broadcast_rx, filter);

    Ok(Sse::new(stream).keep_alive(
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

type EventType = String;
type EventFilter = HashSet<EventType>;

fn parse_event_filter(types: Option<String>) -> Option<EventFilter> {
    types.map(|t| t.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
}

pub async fn get_replay_events(
    event_hub: &EventHub,
    cursor: Option<u64>,
) -> Result<Vec<Event>, EventsErr> {
    match cursor {
        Some(c) => {
            event_hub.replay_after(c).await
        }
        None => Ok(Vec::new())
    }
}

fn new_event_stream(
    replays: Vec<Event>,
    broadcast_rx: broadcast::Receiver<Event>,
    filter: Option<EventFilter>,
) -> impl Stream<Item = Result<SseEvent, Infallible>> {
    let last_replayed_id = replays.last().map(|e| e.id).unwrap_or(0);

    // replay then live, with type filtering and SSE formatting applied uniformly
    let replay_stream = tokio_stream::iter(replays);
    let live_stream = BroadcastStream::new(broadcast_rx)
        .filter_map(|result| result.ok())
        .filter(move |event: &Event| event.id > last_replayed_id);

    replay_stream
        .chain(live_stream)
        .filter_map(move |event: Event| {
            if let Some(ref filter) = filter {
                if !filter.contains(&event.event_type) {
                    return None;
                }
            }
            let json = serde_json::to_string(&event).ok()?;
            Some(Ok::<_, Infallible>(
                SseEvent::default()
                    .id(event.id.to_string())
                    .event(event.event_type.clone())
                    .data(json),
            ))
        })
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
