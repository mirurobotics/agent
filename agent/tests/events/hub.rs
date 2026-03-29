// internal crates
use miru_agent::events::hub::{EventHub, SpawnOptions};
use miru_agent::events::model::{EventArgs, DEPLOYMENT_DEPLOYED_BETA1};
use miru_agent::filesys;

// external crates
use chrono::Utc;

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
    let (hub, _handle) = EventHub::spawn(log_file, SpawnOptions::default()).unwrap();
    (dir, hub)
}

// ========================= PUBLISH ========================= //

mod publish {
    use super::*;

    #[tokio::test]
    async fn returns_envelope_with_monotonic_ids() {
        let (_dir, hub) = make_hub("hub_publish_mono").await;

        let e1 = hub.publish(make_event("test.a")).await.unwrap();
        let e2 = hub.publish(make_event("test.b")).await.unwrap();
        let e3 = hub.publish(make_event("test.c")).await.unwrap();

        assert_eq!(e1.id, 1);
        assert_eq!(e2.id, 2);
        assert_eq!(e3.id, 3);
    }

    #[tokio::test]
    async fn preserves_event_type() {
        let (_dir, hub) = make_hub("hub_publish_type").await;

        let env = hub
            .publish(make_event(DEPLOYMENT_DEPLOYED_BETA1))
            .await
            .unwrap();
        assert_eq!(env.event_type, DEPLOYMENT_DEPLOYED_BETA1);
    }

    #[tokio::test]
    async fn try_publish_does_not_panic_on_success() {
        let (_dir, hub) = make_hub("hub_try_publish").await;
        hub.try_publish(make_event("test.ok")).await;
        // no panic = pass
    }
}

// ========================= REPLAY ========================= //

mod replay {
    use super::*;

    #[tokio::test]
    async fn replays_all_from_zero() {
        let (_dir, hub) = make_hub("hub_replay_zero").await;

        hub.publish(make_event("a")).await.unwrap();
        hub.publish(make_event("b")).await.unwrap();

        let events = hub.replay_after(0).await.unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].id, 1);
        assert_eq!(events[1].id, 2);
    }

    #[tokio::test]
    async fn replays_after_cursor() {
        let (_dir, hub) = make_hub("hub_replay_cursor").await;

        hub.publish(make_event("a")).await.unwrap();
        hub.publish(make_event("b")).await.unwrap();
        hub.publish(make_event("c")).await.unwrap();

        let events = hub.replay_after(1).await.unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].id, 2);
        assert_eq!(events[1].id, 3);
    }

    #[tokio::test]
    async fn replays_empty_when_cursor_at_latest() {
        let (_dir, hub) = make_hub("hub_replay_empty").await;

        hub.publish(make_event("a")).await.unwrap();

        let events = hub.replay_after(1).await.unwrap();
        assert!(events.is_empty());
    }
}

// ========================= SUBSCRIBE ========================= //

mod subscribe {
    use super::*;

    #[tokio::test]
    async fn receives_live_events() {
        let (_dir, hub) = make_hub("hub_sub_live").await;
        let mut rx = hub.subscribe();

        let published = hub.publish(make_event("live.event")).await.unwrap();
        let received = rx.recv().await.unwrap();

        assert_eq!(received.id, published.id);
        assert_eq!(received.event_type, "live.event");
    }

    #[tokio::test]
    async fn receives_multiple_events_in_order() {
        let (_dir, hub) = make_hub("hub_sub_order").await;
        let mut rx = hub.subscribe();

        hub.publish(make_event("first")).await.unwrap();
        hub.publish(make_event("second")).await.unwrap();

        let e1 = rx.recv().await.unwrap();
        let e2 = rx.recv().await.unwrap();

        assert_eq!(e1.event_type, "first");
        assert_eq!(e2.event_type, "second");
        assert!(e2.id > e1.id);
    }
}

// ========================= SHUTDOWN ========================= //

mod shutdown {
    use super::*;

    #[tokio::test]
    async fn shutdown_completes_without_error() {
        let (_dir, hub) = make_hub("hub_shutdown").await;
        hub.publish(make_event("before_shutdown")).await.unwrap();
        hub.shutdown().await.unwrap();
    }
}

// ========================= PERSISTENCE ========================= //

mod persistence {
    use super::*;

    #[tokio::test]
    async fn events_survive_hub_restart() {
        let dir = filesys::Dir::create_temp_dir("hub_persist").await.unwrap();
        let log_file = dir.file("events.jsonl");

        // first hub: publish events
        {
            let (hub, handle) = EventHub::spawn(log_file.clone(), SpawnOptions::default()).unwrap();
            hub.publish(make_event("first")).await.unwrap();
            hub.publish(make_event("second")).await.unwrap();
            hub.shutdown().await.unwrap();
            handle.await.unwrap();
        }

        // second hub: should see persisted events
        {
            let (hub, _handle) = EventHub::spawn(log_file, SpawnOptions::default()).unwrap();
            let events = hub.replay_after(0).await.unwrap();
            assert_eq!(events.len(), 2);
            assert_eq!(events[0].event_type, "first");
            assert_eq!(events[1].event_type, "second");

            // new event should continue from id 3
            let e3 = hub.publish(make_event("third")).await.unwrap();
            assert_eq!(e3.id, 3);
        }
    }
}
