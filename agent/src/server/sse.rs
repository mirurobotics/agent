// standard library
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

// internal crates
use super::state::State;
use crate::events;
use crate::events::errors::MalformedCursorErr;

// external crates
use axum::{
    extract::{Query, State as AxumState},
    http::{HeaderMap, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    Json,
};
use futures::stream::{self, Stream, StreamExt};
use openapi_server::models as openapi;
use serde::Deserialize;
use tracing::error;

const DEFAULT_REPLAY_LIMIT: usize = 1000;
const HEARTBEAT_INTERVAL_SECS: u64 = 30;
const LAST_EVENT_ID_HEADER: &str = "Last-Event-ID";

type ErrorResponse = (StatusCode, Json<openapi::ErrorResponse>);

#[derive(Debug, Deserialize)]
pub struct EventsQuery {
    pub after: Option<String>,
    pub limit: Option<usize>,
}

pub async fn events(
    AxumState(state): AxumState<Arc<State>>,
    headers: HeaderMap,
    Query(query): Query<EventsQuery>,
) -> impl IntoResponse {
    let cursor = match resolve_cursor(&query, &headers) {
        Ok(cursor) => cursor,
        Err(err) => return Err(err),
    };
    let limit = replay_limit(query.limit);

    // subscribe before replay so the receiver buffers events published during
    // the replay read; dedup logic in live_event_stream skips overlap
    let live_rx = state.event_hub.subscribe();

    let replay = match replay_events(state.event_hub.as_ref(), cursor, limit) {
        Ok(replay) => replay,
        Err(err) => return Err(err),
    };

    // track the last replayed ID so we can skip duplicates from the broadcast
    let last_replay_id = replay.last().map(|e| e.id).unwrap_or(cursor);

    // build the SSE stream: replay first, then live
    let replay_stream = stream::iter(replay.into_iter().map(|env| Ok(envelope_to_event(&env))));

    let live_stream = live_event_stream(live_rx, last_replay_id);

    let combined = replay_stream.chain(live_stream);

    Ok(Sse::new(combined).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(HEARTBEAT_INTERVAL_SECS))
            .text("heartbeat"),
    ))
}

fn resolve_cursor(query: &EventsQuery, headers: &HeaderMap) -> Result<u64, ErrorResponse> {
    let cursor = query.after.clone().or_else(|| {
        headers
            .get(LAST_EVENT_ID_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(ToString::to_string)
    });

    parse_cursor(cursor).map_err(|e| bad_request(e))
}

fn parse_cursor(cursor: Option<String>) -> Result<u64, MalformedCursorErr> {
    match cursor {
        Some(cursor) if cursor.is_empty() => Ok(0),
        Some(cursor) => cursor
            .parse::<u64>()
            .map_err(|_| MalformedCursorErr { value: cursor }),
        None => Ok(0),
    }
}

fn replay_limit(limit: Option<usize>) -> usize {
    limit
        .unwrap_or(DEFAULT_REPLAY_LIMIT)
        .min(DEFAULT_REPLAY_LIMIT)
}

fn replay_events(
    event_hub: &events::EventHub,
    cursor: u64,
    limit: usize,
) -> Result<Vec<events::Envelope>, ErrorResponse> {
    match event_hub.replay_after(cursor, limit) {
        Ok(events) => Ok(events),
        Err(events::EventErr::CursorExpiredErr(e)) => {
            Err((StatusCode::GONE, Json(to_error_response(e))))
        }
        Err(e) => {
            error!("failed to replay events: {e}");
            Err(internal_server_error("failed to replay events"))
        }
    }
}

fn live_event_stream(
    rx: tokio::sync::broadcast::Receiver<events::Envelope>,
    after_id: u64,
) -> impl Stream<Item = Result<Event, Infallible>> {
    stream::unfold((rx, after_id), |(mut rx, after_id)| async move {
        loop {
            match rx.recv().await {
                Ok(env) => {
                    // skip events already sent in replay
                    if env.id <= after_id {
                        continue;
                    }
                    return Some((Ok(envelope_to_event(&env)), (rx, env.id)));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    error!("SSE subscriber lagged by {n} events");
                    // continue receiving; client may miss some events
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    return None;
                }
            }
        }
    })
}

fn bad_request(e: impl crate::errors::Error) -> ErrorResponse {
    (StatusCode::BAD_REQUEST, Json(to_error_response(e)))
}

fn internal_server_error(message: impl Into<String>) -> ErrorResponse {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(openapi::ErrorResponse {
            error: Box::new(openapi::Error {
                code: "internal_server_error".to_string(),
                params: Default::default(),
                message: message.into(),
            }),
        }),
    )
}

fn envelope_to_event(env: &events::Envelope) -> Event {
    let data = serde_json::to_string(env).unwrap_or_default();
    Event::default()
        .id(env.id.to_string())
        .event(env.event_type.clone())
        .data(data)
}

fn to_error_response(e: impl crate::errors::Error) -> openapi::ErrorResponse {
    let params = e
        .params()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    openapi::ErrorResponse {
        error: Box::new(openapi::Error {
            code: e.code().as_str().to_string(),
            params,
            message: e.to_string(),
        }),
    }
}
