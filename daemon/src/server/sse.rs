// standard crates
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::events::{
    errors::{EventsErr, MalformedCursorErr},
    model::{Event, EventTypeFilter},
};
use crate::server::{request, state::State};
use crate::services::{events as events_svc, ServiceErr};
use crate::trace;

// external crates
use axum::{
    http::HeaderMap,
    response::{
        sse::{Event as SseEvent, KeepAlive, Sse},
        IntoResponse,
    },
};
use serde::Deserialize;
use tokio_stream::StreamExt;
use tracing::warn;

#[derive(Deserialize)]
pub struct EventsQuery {
    pub after: Option<String>,
    pub types: Option<String>,
}

/// The SSE stream of events, each one's data built by the Device API
/// version's `to_sse_data`.
pub(crate) async fn event_stream(
    state: Arc<State>,
    params: EventsQuery,
    headers: HeaderMap,
    to_sse_data: fn(&Event) -> Result<String, EventsErr>,
) -> Result<impl IntoResponse, ServiceErr> {
    let cursor = resolve_cursor(&params, &headers)?;
    let filter = parse_event_filter(params.types);

    let stream = events_svc::subscribe(&state.event_hub, cursor, filter).await?;

    let sse_stream = stream.filter_map(move |event| {
        let json = to_sse_data(&event);
        match json {
            Ok(json) => Some(Ok::<_, Infallible>(
                SseEvent::default()
                    .id(event.id.to_string())
                    .event(event.event_type.clone())
                    .data(json),
            )),
            Err(e) => {
                warn!("failed to convert event {}: {e}", event.id);
                None
            }
        }
    });

    // Emit an immediate heartbeat comment so clients get a byte on the wire as
    // soon as the connection is established, even when no event is ready yet.
    // The periodic KeepAlive below only fires after an idle interval, leaving
    // an otherwise silent gap at the start of the stream.
    let initial_heartbeat = tokio_stream::once(Ok::<_, Infallible>(
        SseEvent::default().comment("heartbeat"),
    ));
    let sse_stream = initial_heartbeat.chain(sse_stream);

    let mut shutdown_rx = state.shutdown_tx.subscribe();
    let sse_stream = futures::StreamExt::take_until(sse_stream, async move {
        let _ = shutdown_rx.recv().await;
    });

    Ok(Sse::new(sse_stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(30))
            .text("heartbeat"),
    ))
}

fn resolve_cursor(params: &EventsQuery, headers: &HeaderMap) -> Result<Option<i64>, EventsErr> {
    if let Some(after) = &params.after {
        return after
            .parse::<i64>()
            .map(Some)
            .map_err(|_| EventsErr::MalformedCursorErr(MalformedCursorErr { trace: trace!() }));
    }
    if let Some(last_event_id) = headers.get("Last-Event-ID") {
        let s = last_event_id
            .to_str()
            .map_err(|_| EventsErr::MalformedCursorErr(MalformedCursorErr { trace: trace!() }))?;
        return s
            .parse::<i64>()
            .map(Some)
            .map_err(|_| EventsErr::MalformedCursorErr(MalformedCursorErr { trace: trace!() }));
    }
    Ok(None)
}

fn parse_event_filter(types: Option<String>) -> Option<EventTypeFilter> {
    let set: EventTypeFilter = request::split_list(types).into_iter().collect();
    (!set.is_empty()).then_some(set)
}
