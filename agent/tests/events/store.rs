// internal crates
use miru_agent::events::errors::EventsErr;
use miru_agent::events::model::{Event, EventArgs, DEPLOYMENT_DEPLOYED};
use miru_agent::events::store::{EventStore, DEFAULT_MAX_RETAINED};
use miru_agent::filesys::{self, WriteOptions};

// external crates
use chrono::Utc;

fn make_event(event_type: &str) -> EventArgs {
    EventArgs {
        event_type: event_type.to_string(),
        occurred_at: Utc::now(),
        data: serde_json::json!({"test": true}),
    }
}

async fn make_store(dir: &filesys::Dir, max_retained: usize) -> EventStore {
    let log_file = dir.file("events.jsonl");
    EventStore::init(log_file, max_retained).await.unwrap()
}

// ========================= INIT ========================= //

mod init {
    use super::*;

    #[tokio::test]
    async fn empty_dir_starts_at_id_1() {
        let dir = filesys::Dir::create_temp_dir("ev_init_empty")
            .await
            .unwrap();
        let store = make_store(&dir, DEFAULT_MAX_RETAINED).await;
        assert_eq!(store.earliest_id(), None);
        assert_eq!(store.latest_id(), None);
    }

    #[tokio::test]
    async fn loads_existing_log() {
        let dir = filesys::Dir::create_temp_dir("ev_init_load").await.unwrap();
        let log_file = dir.file("events.jsonl");

        // write two events manually
        let e1 = Event {
            id: 5,
            event_type: "test.a".to_string(),
            occurred_at: Utc::now(),
            data: serde_json::json!({}),
        };
        let e2 = Event {
            id: 10,
            event_type: "test.b".to_string(),
            occurred_at: Utc::now(),
            data: serde_json::json!({}),
        };

        let content = format!(
            "{}\n{}\n",
            serde_json::to_string(&e1).unwrap(),
            serde_json::to_string(&e2).unwrap(),
        );
        log_file
            .write_string(&content, WriteOptions::default())
            .await
            .unwrap();

        let store = make_store(&dir, DEFAULT_MAX_RETAINED).await;
        assert_eq!(store.earliest_id(), Some(5));
        assert_eq!(store.latest_id(), Some(10));

        // next append should get id 11
        let mut store = store;
        let env = store.append(make_event("test.c")).await.unwrap();
        assert_eq!(env.id, 11);
    }

    #[tokio::test]
    async fn skips_malformed_lines() {
        let dir = filesys::Dir::create_temp_dir("ev_init_malformed")
            .await
            .unwrap();
        let log_file = dir.file("events.jsonl");

        let valid = Event {
            id: 3,
            event_type: "test.ok".to_string(),
            occurred_at: Utc::now(),
            data: serde_json::json!({}),
        };

        let content = format!(
            "not valid json\n\n{}\n{{\"broken\": true}}\n",
            serde_json::to_string(&valid).unwrap(),
        );
        log_file
            .write_string(&content, WriteOptions::default())
            .await
            .unwrap();

        let store = make_store(&dir, DEFAULT_MAX_RETAINED).await;
        assert_eq!(store.earliest_id(), Some(3));
        assert_eq!(store.latest_id(), Some(3));
    }

    #[tokio::test]
    async fn all_malformed_lines_produces_empty_store() {
        let dir = filesys::Dir::create_temp_dir("ev_all_malformed")
            .await
            .unwrap();
        let log_file = dir.file("events.jsonl");

        log_file
            .write_string(
                "not json at all\n{\"broken\": true}\nalso garbage\n",
                WriteOptions::default(),
            )
            .await
            .unwrap();

        let mut store = EventStore::init(log_file, DEFAULT_MAX_RETAINED)
            .await
            .unwrap();
        assert_eq!(store.earliest_id(), None);
        assert_eq!(store.latest_id(), None);

        // first append should get id=1
        let e = store.append(make_event("first")).await.unwrap();
        assert_eq!(
            e.id, 1,
            "next_event_id should start at 1 when all lines were malformed"
        );
    }
}

// ========================= APPEND ========================= //

mod append {
    use super::*;

    #[tokio::test]
    async fn assigns_monotonic_ids() {
        let dir = filesys::Dir::create_temp_dir("ev_append_mono")
            .await
            .unwrap();
        let mut store = make_store(&dir, DEFAULT_MAX_RETAINED).await;

        let e1 = store.append(make_event("test.a")).await.unwrap();
        let e2 = store.append(make_event("test.b")).await.unwrap();
        let e3 = store.append(make_event("test.c")).await.unwrap();

        assert_eq!(e1.id, 1);
        assert_eq!(e2.id, 2);
        assert_eq!(e3.id, 3);
    }

    #[tokio::test]
    async fn persists_to_disk() {
        let dir = filesys::Dir::create_temp_dir("ev_append_disk")
            .await
            .unwrap();
        let log_file = dir.file("events.jsonl");

        {
            let mut store = make_store(&dir, DEFAULT_MAX_RETAINED).await;
            store.append(make_event("test.a")).await.unwrap();
            store.append(make_event("test.b")).await.unwrap();
        }

        // reload from disk
        let store = EventStore::init(log_file, DEFAULT_MAX_RETAINED)
            .await
            .unwrap();
        assert_eq!(store.earliest_id(), Some(1));
        assert_eq!(store.latest_id(), Some(2));
    }

    #[tokio::test]
    async fn preserves_event_type_and_data() {
        let dir = filesys::Dir::create_temp_dir("ev_append_data")
            .await
            .unwrap();
        let mut store = make_store(&dir, DEFAULT_MAX_RETAINED).await;

        let event_args = EventArgs {
            event_type: DEPLOYMENT_DEPLOYED.to_string(),
            occurred_at: Utc::now(),
            data: serde_json::json!({"deployment_id": "dpl-1", "activity_status": "deployed"}),
        };

        let event = store.append(event_args).await.unwrap();
        assert_eq!(event.event_type, DEPLOYMENT_DEPLOYED);
        assert_eq!(event.data["deployment_id"], "dpl-1");
        assert_eq!(event.data["activity_status"], "deployed");
    }
}

// ========================= REPLAY ========================= //

mod replay {
    use super::*;

    #[tokio::test]
    async fn cursor_zero_returns_all() {
        let dir = filesys::Dir::create_temp_dir("ev_replay_zero")
            .await
            .unwrap();
        let mut store = make_store(&dir, DEFAULT_MAX_RETAINED).await;

        store.append(make_event("a")).await.unwrap();
        store.append(make_event("b")).await.unwrap();
        store.append(make_event("c")).await.unwrap();

        let events = store.replay_after(0).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].id, 1);
        assert_eq!(events[2].id, 3);
    }

    #[tokio::test]
    async fn returns_events_after_cursor() {
        let dir = filesys::Dir::create_temp_dir("ev_replay_after")
            .await
            .unwrap();
        let mut store = make_store(&dir, DEFAULT_MAX_RETAINED).await;

        store.append(make_event("a")).await.unwrap();
        store.append(make_event("b")).await.unwrap();
        store.append(make_event("c")).await.unwrap();

        let events = store.replay_after(1).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].id, 2);
        assert_eq!(events[1].id, 3);
    }

    #[tokio::test]
    async fn cursor_at_latest_returns_empty() {
        let dir = filesys::Dir::create_temp_dir("ev_replay_latest")
            .await
            .unwrap();
        let mut store = make_store(&dir, DEFAULT_MAX_RETAINED).await;

        store.append(make_event("a")).await.unwrap();
        store.append(make_event("b")).await.unwrap();

        let events = store.replay_after(2).unwrap();
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn cursor_beyond_latest_returns_empty() {
        let dir = filesys::Dir::create_temp_dir("ev_replay_beyond")
            .await
            .unwrap();
        let mut store = make_store(&dir, DEFAULT_MAX_RETAINED).await;

        store.append(make_event("a")).await.unwrap();

        let events = store.replay_after(999).unwrap();
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn cursor_zero_on_empty_store_returns_empty() {
        let dir = filesys::Dir::create_temp_dir("ev_empty_replay")
            .await
            .unwrap();
        let store = make_store(&dir, DEFAULT_MAX_RETAINED).await;

        let result = store.replay_after(0);
        assert!(
            result.is_ok(),
            "replay_after(0) on empty store should succeed"
        );
        assert!(
            result.unwrap().is_empty(),
            "empty store should return empty replay"
        );
    }

    #[tokio::test]
    async fn expired_cursor_returns_error() {
        let dir = filesys::Dir::create_temp_dir("ev_replay_expired")
            .await
            .unwrap();
        // small max_retained to force compaction
        let mut store = make_store(&dir, 4).await;

        // append enough to trigger compaction (> max_retained)
        for i in 0..5 {
            store.append(make_event(&format!("evt-{i}"))).await.unwrap();
        }

        // after compaction, earliest id should be > 1
        let earliest = store.earliest_id().unwrap();
        assert!(
            earliest > 1,
            "expected compaction to have removed early events"
        );

        // cursor before earliest should fail
        let result = store.replay_after(1);
        assert!(
            matches!(result, Err(EventsErr::CursorExpiredErr(_))),
            "expected CursorExpiredErr, got: {result:?}"
        );
    }
}

// ========================= COMPACTION ========================= //

mod compaction {
    use super::*;

    #[tokio::test]
    async fn compacts_when_exceeding_max_retained() {
        let dir = filesys::Dir::create_temp_dir("ev_compact").await.unwrap();
        let max_retained = 10;
        let mut store = make_store(&dir, max_retained).await;

        // append max_retained + 1 events to trigger compaction
        for i in 0..=max_retained {
            store.append(make_event(&format!("evt-{i}"))).await.unwrap();
        }

        // compaction keeps 80% of max_retained events
        let keep_count = (max_retained * 80) / 100;
        let expected_earliest = (max_retained + 1) - keep_count + 1;
        assert_eq!(store.earliest_id(), Some(expected_earliest as i64));
        assert_eq!(store.latest_id(), Some((max_retained + 1) as i64));
    }

    #[tokio::test]
    async fn compacted_log_survives_reload() {
        let dir = filesys::Dir::create_temp_dir("ev_compact_reload")
            .await
            .unwrap();
        let max_retained = 6;
        let mut store = make_store(&dir, max_retained).await;

        for i in 0..8 {
            store.append(make_event(&format!("evt-{i}"))).await.unwrap();
        }

        let earliest_before = store.earliest_id().unwrap();
        let latest_before = store.latest_id().unwrap();

        // reload from disk
        let log_file = dir.file("events.jsonl");
        let reloaded = EventStore::init(log_file, max_retained).await.unwrap();
        assert_eq!(reloaded.earliest_id(), Some(earliest_before));
        assert_eq!(reloaded.latest_id(), Some(latest_before));
    }

    #[tokio::test]
    async fn append_after_compaction_continues_ids() {
        let dir = filesys::Dir::create_temp_dir("ev_compact_ids")
            .await
            .unwrap();
        let mut store = make_store(&dir, 4).await;

        // append 5 events (triggers compaction at > 4)
        for i in 0..5 {
            store.append(make_event(&format!("evt-{i}"))).await.unwrap();
        }

        // 6th event should get id=6, not restart
        let e6 = store.append(make_event("after-compact")).await.unwrap();
        assert_eq!(
            e6.id, 6,
            "IDs should continue monotonically after compaction"
        );
    }
}
