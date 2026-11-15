// internal crates
use crate::events::{
    hub::EventHub,
    model::{Event, EventTypeFilter},
};
use crate::services::errors::ServiceErr;

// external crates
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use tracing::warn;

pub async fn subscribe(
    event_hub: &EventHub,
    cursor: Option<i64>,
    filter: Option<EventTypeFilter>,
) -> Result<impl tokio_stream::Stream<Item = Event> + Send, ServiceErr> {
    // subscribe BEFORE replay to prevent gaps
    let broadcast_rx = event_hub.subscribe();

    let replays = match cursor {
        Some(c) => event_hub.replay_after(c).await?,
        None => Vec::new(),
    };

    let last_replayed_id = replays.last().map(|e| e.id).unwrap_or(0);

    let replay_stream = tokio_stream::iter(replays);
    let live_stream = BroadcastStream::new(broadcast_rx)
        .map(|result| {
            result.map_err(|e| {
                warn!("SSE client fell behind the event broadcast buffer, closing connection so it can reconnect and replay missed events: {e}");
            })
        })
        // cut the stream off if the result is not vali
        .take_while(|result| result.is_ok())
        .filter_map(|result| result.ok())
        .filter(move |event| event.id > last_replayed_id);

    let stream = replay_stream
        .chain(live_stream)
        .filter(move |event| match &filter {
            Some(f) => f.contains(&event.event_type),
            None => true,
        });

    Ok(stream)
}
