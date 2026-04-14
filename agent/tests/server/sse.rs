// standard crates
use std::sync::Arc;
use std::time::Duration;

// internal crates
use crate::http::mock::MockClient;
use crate::sync::syncer::{create_storage, create_token_manager};
use miru_agent::activity;
use miru_agent::events::hub::{EventHub, SpawnOptions};
use miru_agent::events::model::EventArgs;
use miru_agent::filesys;
use miru_agent::server::{serve, State};
use miru_agent::sync::Syncer;

// external crates
use axum::body::{self, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use chrono::Utc;
use http_body_util::BodyExt;
use tokio::sync::{broadcast, mpsc};
use tower::ServiceExt;

struct Fixture {
    state: Arc<State>,
    app: Router,
    shutdown_tx: broadcast::Sender<()>,
    _dir: filesys::Dir,
}

impl Fixture {
    async fn new(name: &str) -> Self {
        Self::with_hub_opts(name, SpawnOptions::default()).await
    }

    async fn with_hub_opts(name: &str, opts: SpawnOptions) -> Self {
        let dir = filesys::Dir::create_temp_dir(name).await.unwrap();
        let storage = Arc::new(create_storage(&dir).await);
        let http_client = Arc::new(MockClient::default());
        let (token_mngr, _handle) = create_token_manager(&dir, http_client.clone()).await;
        let (sender, _receiver) = mpsc::channel(1);
        let syncer = Arc::new(Syncer::new(sender));
        let activity_tracker = Arc::new(activity::Tracker::new());

        let real_http_client =
            Arc::new(miru_agent::http::Client::new("http://localhost:1").unwrap());

        let log_file = dir.file("events.jsonl");
        let (event_hub, _handle) = EventHub::spawn(log_file, opts).await.unwrap();

        let (shutdown_tx, _) = broadcast::channel::<()>(1);

        let state = Arc::new(State::new(
            storage,
            real_http_client,
            syncer,
            Arc::new(token_mngr),
            activity_tracker,
            event_hub,
            shutdown_tx.clone(),
        ));

        let app = serve::routes(state.clone());

        Self {
            state,
            app,
            shutdown_tx,
            _dir: dir,
        }
    }

    /// Send a request and read the full (non-streaming) response body.
    async fn request(&self, req: Request<Body>) -> (StatusCode, Vec<u8>) {
        let response = self.app.clone().oneshot(req).await.unwrap();
        let status = response.status();
        let bytes = body::to_bytes(response.into_body(), 65536).await.unwrap();
        (status, bytes.to_vec())
    }

    /// Send a request and collect SSE frames within a timeout.
    /// SSE streams never end, so we read frames until the timeout expires.
    async fn request_sse(&self, req: Request<Body>, timeout: Duration) -> (StatusCode, String) {
        let response = self.app.clone().oneshot(req).await.unwrap();
        let status = response.status();

        if !status.is_success() {
            let bytes = body::to_bytes(response.into_body(), 65536).await.unwrap();
            return (
                status,
                String::from_utf8(bytes.to_vec()).unwrap_or_default(),
            );
        }

        let mut body = response.into_body();
        let mut collected = Vec::new();

        // Read frames with a timeout — SSE streams never close on their own
        let _ = tokio::time::timeout(timeout, async {
            while let Some(Ok(frame)) = body.frame().await {
                if let Some(data) = frame.data_ref() {
                    collected.extend_from_slice(data);
                }
            }
        })
        .await;

        (status, String::from_utf8(collected).unwrap_or_default())
    }

    fn event_hub(&self) -> &EventHub {
        &self.state.event_hub
    }
}

fn make_event(event_type: &str) -> EventArgs {
    EventArgs {
        event_type: event_type.to_string(),
        occurred_at: Utc::now(),
        data: serde_json::json!({"test": true}),
    }
}

// ========================= CURSOR HANDLING ========================= //

mod cursor {
    use super::*;
    use device_api::models as openapi;

    #[tokio::test]
    async fn malformed_after_param_returns_400() {
        let f = Fixture::new("sse_bad_cursor").await;

        let req = Request::builder()
            .uri("/v0.2/events?after=notanumber")
            .header("Accept", "text/event-stream")
            .body(Body::empty())
            .unwrap();

        let (status, bytes) = f.request(req).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let actual: openapi::ErrorResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(actual.error.code, "malformed_cursor");
    }

    #[tokio::test]
    async fn malformed_last_event_id_returns_400() {
        let f = Fixture::new("sse_bad_lei").await;

        let req = Request::builder()
            .uri("/v0.2/events")
            .header("Last-Event-ID", "invalid")
            .body(Body::empty())
            .unwrap();

        let (status, bytes) = f.request(req).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let actual: openapi::ErrorResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(actual.error.code, "malformed_cursor");
    }

    #[tokio::test]
    async fn expired_cursor_returns_410() {
        let opts = SpawnOptions {
            max_retained: 4,
            ..SpawnOptions::default()
        };
        let f = Fixture::with_hub_opts("sse_expired_cursor", opts).await;

        // Publish enough events to trigger compaction
        for i in 0..6 {
            f.event_hub()
                .publish(make_event(&format!("evt-{i}")))
                .await
                .unwrap();
        }

        // Cursor 1 should now be expired
        let req = Request::builder()
            .uri("/v0.2/events?after=1")
            .header("Accept", "text/event-stream")
            .body(Body::empty())
            .unwrap();

        let (status, bytes) = f.request(req).await;
        assert_eq!(status, StatusCode::GONE);
        let actual: openapi::ErrorResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(actual.error.code, "cursor_expired");
    }

    #[tokio::test]
    async fn after_param_takes_precedence_over_last_event_id() {
        let f = Fixture::new("sse_after_precedence").await;

        // Publish 3 events
        f.event_hub().publish(make_event("a")).await.unwrap();
        f.event_hub().publish(make_event("b")).await.unwrap();
        f.event_hub().publish(make_event("c")).await.unwrap();

        // after=2 should return event 3, ignoring Last-Event-ID: 0
        let req = Request::builder()
            .uri("/v0.2/events?after=2")
            .header("Last-Event-ID", "0")
            .header("Accept", "text/event-stream")
            .body(Body::empty())
            .unwrap();

        let (status, body) = f.request_sse(req, Duration::from_millis(200)).await;
        assert_eq!(status, StatusCode::OK);

        // should contain event id 3 but not 1 or 2
        assert!(
            body.contains("id: 3"),
            "expected event 3 in response, body: {body}"
        );
        assert!(!body.contains("\nid: 1\n"), "should not contain event 1");
        assert!(!body.contains("\nid: 2\n"), "should not contain event 2");
    }

    #[tokio::test]
    async fn non_utf8_last_event_id_returns_400() {
        let f = Fixture::new("sse_non_utf8").await;

        let req = Request::builder()
            .uri("/v0.2/events")
            .header("Last-Event-ID", &b"\xff\xfe"[..])
            .body(Body::empty())
            .unwrap();

        let (status, bytes) = f.request(req).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let actual: openapi::ErrorResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(actual.error.code, "malformed_cursor");
    }
}

// ========================= SSE STREAM ========================= //

mod stream {
    use super::*;

    #[tokio::test]
    async fn returns_200_with_sse_content() {
        // lint:allow(field-by-field-assert)
        let f = Fixture::new("sse_stream_200").await;

        f.event_hub()
            .publish(make_event("test.event"))
            .await
            .unwrap();

        let req = Request::builder()
            .uri("/v0.2/events?after=0")
            .header("Accept", "text/event-stream")
            .body(Body::empty())
            .unwrap();

        let (status, body) = f.request_sse(req, Duration::from_millis(200)).await;
        assert_eq!(status, StatusCode::OK);

        assert!(
            body.contains("id: 1"),
            "expected event id in SSE output, body: {body}"
        );
        assert!(
            body.contains("event: test.event"),
            "expected event type in SSE output, body: {body}"
        );

        // deserialize the data payload into the openapi struct to verify contract conformance
        let data_json = body
            .lines()
            .find(|l| l.starts_with("data: "))
            .expect("expected data: line in SSE output")
            .strip_prefix("data: ")
            .unwrap();
        let event: device_api::models::Event =
            serde_json::from_str(data_json).unwrap_or_else(|e| {
                panic!("data payload must deserialize as device API Event: {e}\nraw: {data_json}")
            });

        assert_eq!(event.object, device_api::models::event::Object::Event);
        assert_eq!(event.id, 1);
        assert_eq!(event.r#type, "test.event");
        assert_eq!(event.data, serde_json::json!({"test": true}));
    }

    #[tokio::test]
    async fn replays_from_last_event_id_header() {
        let f = Fixture::new("sse_last_event_id").await;

        f.event_hub().publish(make_event("first")).await.unwrap();
        f.event_hub().publish(make_event("second")).await.unwrap();
        f.event_hub().publish(make_event("third")).await.unwrap();

        let req = Request::builder()
            .uri("/v0.2/events")
            .header("Last-Event-ID", "1")
            .body(Body::empty())
            .unwrap();

        let (status, body) = f.request_sse(req, Duration::from_millis(200)).await;
        assert_eq!(status, StatusCode::OK);

        assert!(
            !body.contains("\nid: 1\n"),
            "should not contain event 1, body: {body}"
        );
        assert!(body.contains("id: 2"), "expected event 2, body: {body}");
        assert!(body.contains("id: 3"), "expected event 3, body: {body}");
    }

    #[tokio::test]
    async fn no_cursor_skips_replay() {
        let f = Fixture::new("sse_no_cursor_no_replay").await;

        // publish events before connecting
        f.event_hub().publish(make_event("old")).await.unwrap();
        f.event_hub().publish(make_event("old")).await.unwrap();

        // connect without cursor — should NOT replay historical events
        let req = Request::builder()
            .uri("/v0.2/events")
            .header("Accept", "text/event-stream")
            .body(Body::empty())
            .unwrap();

        let (status, body) = f.request_sse(req, Duration::from_millis(200)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            !body.contains("event: old"),
            "fresh connection should not replay historical events, body: {body}"
        );
    }

    #[tokio::test]
    async fn empty_stream_returns_200() {
        let f = Fixture::new("sse_empty").await;

        let req = Request::builder()
            .uri("/v0.2/events")
            .header("Accept", "text/event-stream")
            .body(Body::empty())
            .unwrap();

        let (status, _body) = f.request_sse(req, Duration::from_millis(200)).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn cursor_zero_replays_all() {
        let f = Fixture::new("sse_cursor_zero_all").await;

        f.event_hub().publish(make_event("a")).await.unwrap();
        f.event_hub().publish(make_event("b")).await.unwrap();
        f.event_hub().publish(make_event("c")).await.unwrap();

        let req = Request::builder()
            .uri("/v0.2/events?after=0")
            .header("Accept", "text/event-stream")
            .body(Body::empty())
            .unwrap();

        let (status, body) = f.request_sse(req, Duration::from_millis(200)).await;
        assert_eq!(status, StatusCode::OK);

        assert!(body.contains("id: 1"), "expected event 1, body: {body}");
        assert!(body.contains("id: 2"), "expected event 2, body: {body}");
        assert!(body.contains("id: 3"), "expected event 3, body: {body}");
    }

    #[tokio::test]
    async fn live_events_appear_after_replay() {
        let f = Fixture::new("sse_replay_then_live").await;

        // Publish 2 events before connecting
        f.event_hub().publish(make_event("replay")).await.unwrap();
        f.event_hub().publish(make_event("replay")).await.unwrap();

        // Clone the hub for the spawned task
        let hub = f.event_hub().clone();

        // Spawn a task that publishes a live event after a short delay
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            hub.publish(make_event("live")).await.unwrap();
        });

        let req = Request::builder()
            .uri("/v0.2/events?after=0")
            .header("Accept", "text/event-stream")
            .body(Body::empty())
            .unwrap();

        let (status, body) = f.request_sse(req, Duration::from_millis(300)).await;
        assert_eq!(status, StatusCode::OK);

        // Should contain both replayed events and the live event
        assert!(
            body.contains("id: 1"),
            "expected replayed event 1, body: {body}"
        );
        assert!(
            body.contains("id: 2"),
            "expected replayed event 2, body: {body}"
        );
        assert!(
            body.contains("id: 3"),
            "expected live event 3, body: {body}"
        );
        assert!(
            body.contains("event: live"),
            "expected live event type, body: {body}"
        );
    }

    #[tokio::test]
    async fn broadcast_lag_closes_stream() {
        let opts = SpawnOptions {
            broadcast_capacity: 2,
            ..SpawnOptions::default()
        };
        let f = Fixture::with_hub_opts("sse_lag_close", opts).await;

        let req = Request::builder()
            .uri("/v0.2/events")
            .header("Accept", "text/event-stream")
            .body(Body::empty())
            .unwrap();

        // Get the response (creates the subscriber) but don't read the body yet
        let response = f.app.clone().oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Flood the broadcast channel while nothing is consuming frames.
        // With broadcast_capacity=2, publishing 10 events guarantees lag.
        for i in 0..10 {
            f.event_hub()
                .publish(make_event(&format!("flood-{i}")))
                .await
                .unwrap();
        }

        // Now read the body — the stream should close without delivering all events
        let mut body_stream = response.into_body();
        let mut collected = Vec::new();
        let _ = tokio::time::timeout(Duration::from_millis(500), async {
            while let Some(Ok(frame)) = body_stream.frame().await {
                if let Some(data) = frame.data_ref() {
                    collected.extend_from_slice(data);
                }
            }
        })
        .await;

        let body = String::from_utf8(collected).unwrap_or_default();
        let event_count = body.matches("\nevent: ").count();
        assert!(
            event_count < 10,
            "stream should have closed due to lag, but got {event_count} events"
        );
    }

    #[tokio::test]
    async fn stream_closes_on_shutdown() {
        let f = Fixture::new("sse_stream_closes_on_shutdown").await;

        // Open an SSE connection but do not read the body yet
        let req = Request::builder()
            .uri("/v0.2/events")
            .header("Accept", "text/event-stream")
            .body(Body::empty())
            .unwrap();
        let response = f.app.clone().oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Signal shutdown
        f.shutdown_tx.send(()).unwrap();

        // The body stream should end on its own promptly — not hang until timeout
        let mut body = response.into_body();
        let result = tokio::time::timeout(Duration::from_millis(500), async {
            while let Some(Ok(_frame)) = body.frame().await {}
        })
        .await;

        assert!(
            result.is_ok(),
            "SSE stream should close promptly after shutdown signal, not hang until timeout"
        );
    }
}

// =========================== FILTER =========================== //

mod filter {
    use super::*;

    #[tokio::test]
    async fn filters_events_by_type() {
        let f = Fixture::new("sse_filter").await;

        f.event_hub().publish(make_event("type.a")).await.unwrap();
        f.event_hub().publish(make_event("type.b")).await.unwrap();
        f.event_hub().publish(make_event("type.a")).await.unwrap();

        let req = Request::builder()
            .uri("/v0.2/events?after=0&types=type.a")
            .header("Accept", "text/event-stream")
            .body(Body::empty())
            .unwrap();

        let (status, body) = f.request_sse(req, Duration::from_millis(200)).await;
        assert_eq!(status, StatusCode::OK);

        assert!(
            body.contains("event: type.a"),
            "expected type.a events, body: {body}"
        );
        assert!(
            !body.contains("event: type.b"),
            "should not contain type.b, body: {body}"
        );
    }

    #[tokio::test]
    async fn multiple_filter() {
        let f = Fixture::new("sse_multi_type").await;

        f.event_hub().publish(make_event("type.a")).await.unwrap();
        f.event_hub().publish(make_event("type.b")).await.unwrap();
        f.event_hub().publish(make_event("type.c")).await.unwrap();

        let req = Request::builder()
            .uri("/v0.2/events?after=0&types=type.a,type.c")
            .header("Accept", "text/event-stream")
            .body(Body::empty())
            .unwrap();

        let (status, body) = f.request_sse(req, Duration::from_millis(200)).await;
        assert_eq!(status, StatusCode::OK);

        assert!(body.contains("event: type.a"), "expected type.a");
        assert!(body.contains("event: type.c"), "expected type.c");
        assert!(!body.contains("event: type.b"), "should not contain type.b");
    }

    #[tokio::test]
    async fn empty_types_param_returns_all_events() {
        let f = Fixture::new("sse_empty_types").await;

        f.event_hub().publish(make_event("type.a")).await.unwrap();

        let req = Request::builder()
            .uri("/v0.2/events?after=0&types=")
            .header("Accept", "text/event-stream")
            .body(Body::empty())
            .unwrap();

        let (status, body) = f.request_sse(req, Duration::from_millis(200)).await;
        assert_eq!(status, StatusCode::OK);

        // empty types= is treated as no filter (returns all events)
        assert!(
            body.contains("event: type.a"),
            "empty types param should return all events, body: {body}"
        );
    }

    #[tokio::test]
    async fn only_commas_types_param_returns_all_events() {
        let f = Fixture::new("sse_commas_types").await;

        f.event_hub().publish(make_event("type.a")).await.unwrap();

        let req = Request::builder()
            .uri("/v0.2/events?after=0&types=,,,")
            .header("Accept", "text/event-stream")
            .body(Body::empty())
            .unwrap();

        let (status, body) = f.request_sse(req, Duration::from_millis(200)).await;
        assert_eq!(status, StatusCode::OK);

        assert!(
            body.contains("event: type.a"),
            "types=,,, should return all events, body: {body}"
        );
    }

    #[tokio::test]
    async fn types_with_whitespace_are_trimmed() {
        let f = Fixture::new("sse_types_ws").await;

        f.event_hub().publish(make_event("type.a")).await.unwrap();
        f.event_hub().publish(make_event("type.b")).await.unwrap();

        let req = Request::builder()
            .uri("/v0.2/events?after=0&types=%20type.a%20,%20type.b%20")
            .header("Accept", "text/event-stream")
            .body(Body::empty())
            .unwrap();

        let (status, body) = f.request_sse(req, Duration::from_millis(200)).await;
        assert_eq!(status, StatusCode::OK);

        assert!(
            body.contains("event: type.a"),
            "type.a should match after trim, body: {body}"
        );
        assert!(
            body.contains("event: type.b"),
            "type.b should match after trim, body: {body}"
        );
    }
}
