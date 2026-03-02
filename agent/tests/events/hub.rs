use std::path::PathBuf;

use chrono::Utc;
use miru_agent::events::{Envelope, EventErr, EventHub};
use miru_agent::filesys::{self, PathExt};

struct Fixture {
    hub: EventHub,
    log_path: PathBuf,
    meta_path: PathBuf,
    _dir: filesys::Dir,
}

impl Fixture {
    async fn new(name: &str) -> Self {
        Self::new_with_max_retained(name, 10_000).await
    }

    async fn new_with_max_retained(name: &str, max_retained: usize) -> Self {
        let dir = filesys::Dir::create_temp_dir(name).await.unwrap();
        let events_dir = dir.subdir("events");
        let log_path = events_dir.file("events.ndjson").path().clone();
        let meta_path = events_dir.file("events.meta.json").path().clone();

        let hub = EventHub::init_with_capacity(&log_path, &meta_path, max_retained, 256).unwrap();

        Self {
            hub,
            log_path,
            meta_path,
            _dir: dir,
        }
    }

    async fn new_from_preseeded(
        name: &str,
        log_lines: &str,
        meta_json: Option<&str>,
        max_retained: usize,
    ) -> Self {
        let dir = filesys::Dir::create_temp_dir(name).await.unwrap();
        let events_dir = dir.subdir("events");
        let log_file = events_dir.file("events.ndjson");
        let meta_file = events_dir.file("events.meta.json");
        let log_path = log_file.path().clone();
        let meta_path = meta_file.path().clone();

        // Create events directory first
        std::fs::create_dir_all(events_dir.path()).unwrap();

        if !log_lines.is_empty() {
            std::fs::write(&log_path, log_lines).unwrap();
        }
        if let Some(meta) = meta_json {
            std::fs::write(&meta_path, meta).unwrap();
        }

        let hub = EventHub::init_with_capacity(&log_path, &meta_path, max_retained, 256).unwrap();

        Self {
            hub,
            log_path,
            meta_path,
            _dir: dir,
        }
    }
}

fn make_envelope() -> Envelope {
    Envelope::sync_completed("dev-1", Utc::now())
}

fn log_line(id: u64) -> String {
    format!(
        r#"{{"id":{id},"type":"sync.completed","schema_version":1,"occurred_at":"2025-01-01T00:00:00Z","device_id":"dev-1","subject":{{"type":"device","id":"dev-1"}},"data":{{"last_synced_at":"2025-01-01T00:00:00Z"}}}}"#
    )
}

mod init {
    use super::*;

    #[tokio::test]
    async fn fresh_start_next_id_is_1() {
        let f = Fixture::new("hub_fresh").await;
        assert_eq!(f.hub.next_id(), 1);
        assert_eq!(f.hub.earliest_id(), None);
    }

    #[tokio::test]
    async fn meta_recovery_restores_next_id() {
        let f = Fixture::new_from_preseeded(
            "hub_meta_recovery",
            "",
            Some(r#"{"next_event_id": 42}"#),
            10_000,
        )
        .await;
        assert_eq!(f.hub.next_id(), 42);
    }

    #[tokio::test]
    async fn stale_meta_corrected_by_log_scan() {
        let log = format!("{}\n", log_line(10));
        let f = Fixture::new_from_preseeded(
            "hub_stale_meta",
            &log,
            Some(r#"{"next_event_id": 2}"#),
            10_000,
        )
        .await;
        assert_eq!(f.hub.next_id(), 11);
    }

    #[tokio::test]
    async fn malformed_meta_falls_back_to_log_scan() {
        let log = format!("{}\n", log_line(5));
        let f =
            Fixture::new_from_preseeded("hub_malformed_meta", &log, Some("not-json"), 10_000).await;
        assert_eq!(f.hub.next_id(), 6);
    }

    #[tokio::test]
    async fn malformed_log_lines_tolerated() {
        let log = format!("{}\ngarbage-line\n", log_line(3));
        let f = Fixture::new_from_preseeded("hub_malformed_log", &log, None, 10_000).await;
        assert_eq!(f.hub.next_id(), 4);
        assert_eq!(f.hub.earliest_id(), Some(3));
    }
}

mod append {
    use super::*;

    #[tokio::test]
    async fn monotonic_id_assignment() {
        let f = Fixture::new("hub_mono_id").await;
        let e1 = f.hub.publish(make_envelope()).unwrap();
        let e2 = f.hub.publish(make_envelope()).unwrap();
        let e3 = f.hub.publish(make_envelope()).unwrap();
        assert_eq!(e1.id, 1);
        assert_eq!(e2.id, 2);
        assert_eq!(e3.id, 3);
    }

    #[tokio::test]
    async fn earliest_id_set_on_first_append_only() {
        let f = Fixture::new("hub_earliest").await;
        f.hub.publish(make_envelope()).unwrap();
        assert_eq!(f.hub.earliest_id(), Some(1));

        f.hub.publish(make_envelope()).unwrap();
        assert_eq!(f.hub.earliest_id(), Some(1));
    }

    #[tokio::test]
    async fn log_file_created_on_first_append() {
        let f = Fixture::new("hub_log_create").await;
        assert!(!f.log_path.exists());

        f.hub.publish(make_envelope()).unwrap();
        assert!(f.log_path.exists());
    }

    #[tokio::test]
    async fn meta_updated_after_each_append() {
        let f = Fixture::new("hub_meta_update").await;

        f.hub.publish(make_envelope()).unwrap();
        let meta: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&f.meta_path).unwrap()).unwrap();
        assert_eq!(meta["next_event_id"], 2);

        f.hub.publish(make_envelope()).unwrap();
        let meta: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&f.meta_path).unwrap()).unwrap();
        assert_eq!(meta["next_event_id"], 3);
    }

    #[tokio::test]
    async fn log_contains_valid_ndjson() {
        let f = Fixture::new("hub_ndjson").await;
        for _ in 0..3 {
            f.hub.publish(make_envelope()).unwrap();
        }

        let contents = std::fs::read_to_string(&f.log_path).unwrap();
        let lines: Vec<&str> = contents.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.len(), 3);

        for (i, line) in lines.iter().enumerate() {
            let env: Envelope = serde_json::from_str(line).unwrap();
            assert_eq!(env.id, (i + 1) as u64);
        }
    }
}

mod compact {
    use super::*;

    #[tokio::test]
    async fn no_op_at_threshold() {
        let f = Fixture::new_with_max_retained("hub_compact_noop", 3).await;
        for _ in 0..3 {
            f.hub.publish(make_envelope()).unwrap();
        }

        let contents = std::fs::read_to_string(&f.log_path).unwrap();
        let lines: Vec<&str> = contents.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.len(), 3);
    }

    #[tokio::test]
    async fn truncates_when_exceeded() {
        let f = Fixture::new_with_max_retained("hub_compact_trunc", 3).await;
        for _ in 0..5 {
            f.hub.publish(make_envelope()).unwrap();
        }

        let contents = std::fs::read_to_string(&f.log_path).unwrap();
        let lines: Vec<&str> = contents.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.len(), 3);

        let ids: Vec<u64> = lines
            .iter()
            .map(|l| serde_json::from_str::<Envelope>(l).unwrap().id)
            .collect();
        assert_eq!(ids, vec![3, 4, 5]);
    }

    #[tokio::test]
    async fn earliest_id_updated_after_compaction() {
        let f = Fixture::new_with_max_retained("hub_compact_earliest", 2).await;
        for _ in 0..4 {
            f.hub.publish(make_envelope()).unwrap();
        }
        assert_eq!(f.hub.earliest_id(), Some(3));
    }

    #[tokio::test]
    async fn large_windows_reclaim_headroom_to_avoid_recompacting_every_append() {
        let f = Fixture::new_with_max_retained("hub_compact_hysteresis", 20).await;

        for _ in 0..21 {
            f.hub.publish(make_envelope()).unwrap();
        }
        let contents = std::fs::read_to_string(&f.log_path).unwrap();
        let lines: Vec<&str> = contents.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.len(), 18);
        let ids: Vec<u64> = lines
            .iter()
            .map(|line| serde_json::from_str::<Envelope>(line).unwrap().id)
            .collect();
        assert_eq!(ids, (4..=21).collect::<Vec<u64>>());

        // The next append should not immediately trigger compaction.
        f.hub.publish(make_envelope()).unwrap();
        let contents = std::fs::read_to_string(&f.log_path).unwrap();
        let lines: Vec<&str> = contents.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.len(), 19);
        let ids: Vec<u64> = lines
            .iter()
            .map(|line| serde_json::from_str::<Envelope>(line).unwrap().id)
            .collect();
        assert_eq!(ids.first(), Some(&4));
        assert_eq!(ids.last(), Some(&22));
    }
}

mod replay {
    use super::*;

    #[tokio::test]
    async fn empty_store_returns_empty() {
        let f = Fixture::new("hub_replay_empty").await;
        let events = f.hub.replay_after(0, 100).unwrap();
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn after_zero_returns_all() {
        let f = Fixture::new("hub_replay_all").await;
        for _ in 0..3 {
            f.hub.publish(make_envelope()).unwrap();
        }
        let events = f.hub.replay_after(0, 100).unwrap();
        assert_eq!(events.len(), 3);
    }

    #[tokio::test]
    async fn after_nonzero_skips_earlier() {
        let f = Fixture::new("hub_replay_skip").await;
        for _ in 0..3 {
            f.hub.publish(make_envelope()).unwrap();
        }
        let events = f.hub.replay_after(2, 100).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, 3);
    }

    #[tokio::test]
    async fn limit_respected() {
        let f = Fixture::new("hub_replay_limit").await;
        for _ in 0..5 {
            f.hub.publish(make_envelope()).unwrap();
        }
        let events = f.hub.replay_after(0, 3).unwrap();
        assert_eq!(events.len(), 3);
    }

    #[tokio::test]
    async fn zero_limit_returns_empty() {
        let f = Fixture::new("hub_replay_zero_limit").await;
        for _ in 0..3 {
            f.hub.publish(make_envelope()).unwrap();
        }
        let events = f.hub.replay_after(0, 0).unwrap();
        assert!(events.is_empty());
    }
}

mod cursor_expiration {
    use super::*;

    #[tokio::test]
    async fn after_zero_bypasses_check() {
        let f = Fixture::new_with_max_retained("hub_cursor_zero", 2).await;
        for _ in 0..4 {
            f.hub.publish(make_envelope()).unwrap();
        }
        // earliest is 3, but after=0 means "from beginning" and should not error
        let result = f.hub.replay_after(0, 100);
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn after_less_than_earliest_returns_err() {
        let f = Fixture::new_with_max_retained("hub_cursor_expired", 2).await;
        for _ in 0..4 {
            f.hub.publish(make_envelope()).unwrap();
        }
        // earliest is 3, cursor 1 should be expired
        let result = f.hub.replay_after(1, 100);
        assert!(result.is_err());
        match result.unwrap_err() {
            EventErr::CursorExpiredErr(e) => {
                assert_eq!(e.cursor, 1);
                assert_eq!(e.earliest, 3);
            }
            other => panic!("expected CursorExpiredErr, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn after_equals_earliest_is_ok() {
        let f = Fixture::new_with_max_retained("hub_cursor_eq", 2).await;
        for _ in 0..4 {
            f.hub.publish(make_envelope()).unwrap();
        }
        // earliest is 3, cursor 3 should be fine (returns events with id > 3)
        let result = f.hub.replay_after(3, 100);
        assert!(result.is_ok());
        let events = result.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, 4);
    }

    #[tokio::test]
    async fn after_one_before_earliest_is_ok() {
        let f = Fixture::new_with_max_retained("hub_cursor_before_earliest", 2).await;
        for _ in 0..4 {
            f.hub.publish(make_envelope()).unwrap();
        }
        // earliest is 3, cursor 2 should return events with id > 2.
        let result = f.hub.replay_after(2, 100);
        assert!(result.is_ok());
        let events = result.unwrap();
        let ids: Vec<u64> = events.iter().map(|event| event.id).collect();
        assert_eq!(ids, vec![3, 4]);
    }

    #[tokio::test]
    async fn empty_store_nonzero_cursor_returns_ok() {
        let f = Fixture::new("hub_cursor_empty").await;
        let result = f.hub.replay_after(5, 100);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }
}

mod broadcast {
    use super::*;

    #[tokio::test]
    async fn publish_reaches_subscriber() {
        let f = Fixture::new("hub_broadcast_sub").await;
        let mut rx = f.hub.subscribe();

        let published = f.hub.publish(make_envelope()).unwrap();
        let received = rx.try_recv().unwrap();
        assert_eq!(received.id, published.id);
    }

    #[tokio::test]
    async fn publish_works_with_no_subscribers() {
        let f = Fixture::new("hub_broadcast_nosub").await;
        // No subscribe() call — should not panic
        let result = f.hub.publish(make_envelope());
        assert!(result.is_ok());
    }
}
