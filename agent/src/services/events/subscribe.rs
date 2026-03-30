// standard crates
use std::collections::HashSet;
use std::pin::Pin;

// internal crates
use crate::events::{hub::EventHub, model::Event};
use crate::services::errors::ServiceErr;

// external crates
use futures::Stream;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

pub async fn subscribe(
    event_hub: &EventHub,
    cursor: Option<u64>,
    filter: Option<HashSet<String>>,
) -> Result<Pin<Box<dyn Stream<Item = Event> + Send>>, ServiceErr> {
    // subscribe BEFORE replay to prevent gaps
    let broadcast_rx = event_hub.subscribe();

    let replays = match cursor {
        Some(c) => event_hub.replay_after(c).await?,
        None => Vec::new(),
    };

    let last_replayed_id = replays.last().map(|e| e.id).unwrap_or(0);

    let replay_stream = tokio_stream::iter(replays);
    let live_stream = BroadcastStream::new(broadcast_rx)
        .filter_map(|result| result.ok())
        .filter(move |event: &Event| event.id > last_replayed_id);

    let stream = replay_stream
        .chain(live_stream)
        .filter(move |event: &Event| match &filter {
            Some(f) => f.contains(&event.event_type),
            None => true,
        });

    Ok(Box::pin(stream))
}
