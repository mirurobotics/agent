// standard crates
use std::collections::HashSet;
use std::time::Duration;

// internal crates
use miru_agent::events::hub::{EventHub, SpawnOptions};
use miru_agent::events::model::EventArgs;
use miru_agent::filesys;
use miru_agent::services::events as events_svc;

// external crates
use chrono::Utc;
use tokio_stream::StreamExt;

fn make_event(event_type: &str) -> EventArgs {
    EventArgs {
        event_type: event_type.to_string(),
        occurred_at: Utc::now(),
        data: serde_json::json!({"test": true}),
    }
}

async fn make_hub(name: &str) -> (filesys::Dir, EventHub) {
    let dir = filesys::Dir::create_temp_dir(name).await.unwrap();
    let log_file = dir.file("events.jsonl");
    let (hub, _handle) = EventHub::spawn(log_file, SpawnOptions::default())
        .await
        .unwrap();
    (dir, hub)
}

async fn make_hub_w_retained(name: &str, max_retained: usize) -> (filesys::Dir, EventHub) {
    let dir = filesys::Dir::create_temp_dir(name).await.unwrap();
    let log_file = dir.file("events.jsonl");
    let opts = SpawnOptions {
        max_retained,
        ..SpawnOptions::default()
    };
    let (hub, _handle) = EventHub::spawn(log_file, opts).await.unwrap();
    (dir, hub)
}

/// Collect up to `n` events from the stream within a timeout.
async fn collect_n<S>(
    stream: &mut S,
    n: usize,
    timeout: Duration,
) -> Vec<miru_agent::events::model::Event>
where
    S: tokio_stream::Stream<Item = miru_agent::events::model::Event> + Unpin,
{
    let mut items = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout;
    for _ in 0..n {
        match tokio::time::timeout_at(deadline, stream.next()).await {
            Ok(Some(event)) => items.push(event),
            _ => break,
        }
    }
    items
}

// ========================= REPLAY ========================= //

mod replay {
    use super::*;

    #[tokio::test]
    async fn cursor_zero_replays_all_events() {
        let (_dir, hub) = make_hub("svc_sub_replay_all").await;

        hub.publish(make_event("a")).await.unwrap();
        hub.publish(make_event("b")).await.unwrap();
        hub.publish(make_event("c")).await.unwrap();

        let mut stream = events_svc::subscribe(&hub, Some(0), None).await.unwrap();
        let items = collect_n(&mut stream, 3, Duration::from_millis(500)).await;

        assert_eq!(items.len(), 3);
        assert_eq!(items[0].id, 1);
        assert_eq!(items[1].id, 2);
        assert_eq!(items[2].id, 3);
    }

    #[tokio::test]
    async fn cursor_after_some_replays_remaining() {
        let (_dir, hub) = make_hub("svc_sub_replay_partial").await;

        hub.publish(make_event("a")).await.unwrap();
        hub.publish(make_event("b")).await.unwrap();
        hub.publish(make_event("c")).await.unwrap();

        let mut stream = events_svc::subscribe(&hub, Some(2), None).await.unwrap();
        let items = collect_n(&mut stream, 1, Duration::from_millis(500)).await;

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, 3);
    }

    #[tokio::test]
    async fn cursor_none_skips_replay() {
        let (_dir, hub) = make_hub("svc_sub_no_replay").await;

        hub.publish(make_event("old")).await.unwrap();
        hub.publish(make_event("old")).await.unwrap();

        let mut stream = events_svc::subscribe(&hub, None, None).await.unwrap();

        // No items should be available (only historical events exist)
        let items = collect_n(&mut stream, 1, Duration::from_millis(200)).await;
        assert!(
            items.is_empty(),
            "cursor=None should not replay historical events"
        );
    }

    #[tokio::test]
    async fn expired_cursor_returns_error() {
        let (_dir, hub) = make_hub_w_retained("svc_sub_expired", 4).await;

        for i in 0..6 {
            hub.publish(make_event(&format!("evt-{i}"))).await.unwrap();
        }

        let result = events_svc::subscribe(&hub, Some(1), None).await;
        assert!(result.is_err(), "expired cursor should return error");
    }
}

// ========================= LIVE ========================= //

mod live {
    use super::*;

    #[tokio::test]
    async fn receives_live_events_after_subscribe() {
        let (_dir, hub) = make_hub("svc_sub_live").await;

        let mut stream = events_svc::subscribe(&hub, None, None).await.unwrap();

        hub.publish(make_event("live.a")).await.unwrap();
        hub.publish(make_event("live.b")).await.unwrap();

        let items = collect_n(&mut stream, 2, Duration::from_millis(500)).await;

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].event_type, "live.a");
        assert_eq!(items[1].event_type, "live.b");
    }

    /// Verifies that replayed events and live events are delivered in order
    /// without duplicates when a live event is published after subscribing.
    ///
    /// Note: the dedup filter (`event.id > last_replayed_id`) in subscribe.rs
    /// only activates when a publish races the internal subscribe→replay window,
    /// which cannot be triggered deterministically in a single-threaded test.
    /// This test validates the replay+live chain produces the correct output.
    #[tokio::test]
    async fn replay_then_live_delivers_all_without_duplicates() {
        let (_dir, hub) = make_hub("svc_sub_dedup").await;

        // Publish 3 events before subscribing
        hub.publish(make_event("a")).await.unwrap();
        hub.publish(make_event("b")).await.unwrap();
        hub.publish(make_event("c")).await.unwrap();

        // Subscribe with cursor=0 replays all 3
        let mut stream = events_svc::subscribe(&hub, Some(0), None).await.unwrap();

        // Publish a 4th event live
        hub.publish(make_event("d")).await.unwrap();

        let items = collect_n(&mut stream, 5, Duration::from_millis(500)).await;

        let ids: Vec<u64> = items.iter().map(|e| e.id).collect();
        assert_eq!(
            ids,
            vec![1, 2, 3, 4],
            "expected no duplicates, got: {ids:?}"
        );
    }

    #[tokio::test]
    async fn post_subscribe_publish_is_captured_by_live_stream() {
        let (_dir, hub) = make_hub("svc_sub_gap").await;

        // Publish 2 events
        hub.publish(make_event("a")).await.unwrap();
        hub.publish(make_event("b")).await.unwrap();

        // Subscribe with cursor=0 — internally subscribes to broadcast THEN replays
        let mut stream = events_svc::subscribe(&hub, Some(0), None).await.unwrap();

        // Publish event 3 after subscribe call but before consuming the stream.
        // The broadcast receiver (created inside subscribe) captures it.
        hub.publish(make_event("c")).await.unwrap();

        // replay yields [1, 2], live yields [3] (chain is deterministic)
        let items = collect_n(&mut stream, 4, Duration::from_millis(500)).await;
        let ids: Vec<u64> = items.iter().map(|e| e.id).collect();
        assert_eq!(
            ids,
            vec![1, 2, 3],
            "expected replay + live event, got: {ids:?}"
        );
    }
}

// =========================== FILTER =========================== //

mod filter {
    use super::*;

    #[tokio::test]
    async fn filters_replay_events_by_type() {
        let (_dir, hub) = make_hub("svc_sub_filter_replay").await;

        hub.publish(make_event("type.a")).await.unwrap();
        hub.publish(make_event("type.b")).await.unwrap();
        hub.publish(make_event("type.a")).await.unwrap();

        let filter = Some(HashSet::from(["type.a".to_string()]));
        let mut stream = events_svc::subscribe(&hub, Some(0), filter).await.unwrap();

        let items = collect_n(&mut stream, 3, Duration::from_millis(500)).await;

        assert_eq!(items.len(), 2, "only type.a events should pass filter");
        assert!(items.iter().all(|e| e.event_type == "type.a"));
    }

    #[tokio::test]
    async fn filters_live_events_by_type() {
        let (_dir, hub) = make_hub("svc_sub_filter_live").await;

        let filter = Some(HashSet::from(["type.a".to_string()]));
        let mut stream = events_svc::subscribe(&hub, None, filter).await.unwrap();

        hub.publish(make_event("type.a")).await.unwrap();
        hub.publish(make_event("type.b")).await.unwrap();

        let items = collect_n(&mut stream, 2, Duration::from_millis(500)).await;

        assert_eq!(items.len(), 1, "only type.a should pass filter");
        assert_eq!(items[0].event_type, "type.a");
    }

    #[tokio::test]
    async fn no_filter_returns_all_types() {
        let (_dir, hub) = make_hub("svc_sub_no_filter").await;

        hub.publish(make_event("type.a")).await.unwrap();
        hub.publish(make_event("type.b")).await.unwrap();
        hub.publish(make_event("type.c")).await.unwrap();

        let mut stream = events_svc::subscribe(&hub, Some(0), None).await.unwrap();
        let items = collect_n(&mut stream, 3, Duration::from_millis(500)).await;

        assert_eq!(items.len(), 3, "no filter should return all events");
        let types: Vec<&str> = items.iter().map(|e| e.event_type.as_str()).collect();
        assert_eq!(types, vec!["type.a", "type.b", "type.c"]);
    }
}

// ========================= ERROR ========================= //

mod error {
    use super::*;

    #[tokio::test]
    async fn broadcast_lag_skips_lost_events() {
        let dir = filesys::Dir::create_temp_dir("svc_sub_lag").await.unwrap();
        let log_file = dir.file("events.jsonl");
        let opts = SpawnOptions {
            broadcast_capacity: 2,
            ..SpawnOptions::default()
        };
        let (hub, _handle) = EventHub::spawn(log_file, opts).await.unwrap();

        // Subscribe first, then flood the broadcast channel
        let mut stream = events_svc::subscribe(&hub, None, None).await.unwrap();

        // Publish many events rapidly — will overflow the broadcast buffer (cap=2)
        for i in 0..10 {
            hub.publish(make_event(&format!("flood-{i}")))
                .await
                .unwrap();
        }

        // Should receive some events without panic
        let items = collect_n(&mut stream, 10, Duration::from_millis(500)).await;
        assert!(
            !items.is_empty(),
            "should receive at least some events despite lag"
        );
        // No panic = pass
    }
}
