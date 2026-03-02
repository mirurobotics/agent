use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use chrono::Utc;
use http_body_util::BodyExt;
use tower::ServiceExt;

use miru_agent::activity;
use miru_agent::events::{Envelope, EventHub};
use miru_agent::filesys::{self, PathExt};
use miru_agent::server::{serve, State};
use miru_agent::sync::Syncer;
use openapi_server::models as openapi;

use crate::http::mock::MockClient;
use crate::sync::syncer::{create_storage, create_token_manager};

use tokio::sync::mpsc;

struct Fixture {
    state: Arc<State>,
    app: Router,
    _dir: filesys::Dir,
}

impl Fixture {
    async fn new(name: &str) -> Self {
        Self::new_with_max_retained(name, 10_000).await
    }

    async fn new_with_max_retained(name: &str, max_retained: usize) -> Self {
        let dir = filesys::Dir::create_temp_dir(name).await.unwrap();
        let storage = Arc::new(create_storage(&dir).await);
        let http_client = Arc::new(MockClient::default());
        let (token_mngr, _handle) = create_token_manager(&dir, http_client.clone()).await;
        let (sender, _receiver) = mpsc::channel(1);
        let syncer = Arc::new(Syncer::new(sender));
        let activity_tracker = Arc::new(activity::Tracker::new());

        let real_http_client =
            Arc::new(miru_agent::http::Client::new("http://localhost:1").unwrap());

        let events_dir = dir.subdir("events");
        let event_hub = Arc::new(
            EventHub::init_with_capacity(
                events_dir.file("events.ndjson").path(),
                events_dir.file("events.meta.json").path(),
                max_retained,
                256,
            )
            .unwrap(),
        );

        let state = Arc::new(State::new(
            storage,
            real_http_client,
            syncer,
            Arc::new(token_mngr),
            activity_tracker,
            event_hub,
        ));

        let app = serve::routes(state.clone());

        Self {
            state,
            app,
            _dir: dir,
        }
    }

    fn publish(&self, env: Envelope) -> Envelope {
        self.state.event_hub.publish(env).unwrap()
    }

    async fn get_sse(&self, uri: &str) -> (StatusCode, Vec<SseEvent>) {
        let response = self
            .app
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = collect_sse_body(response.into_body()).await;
        let events = parse_sse_events(&body);
        (status, events)
    }

    async fn get_sse_with_header(
        &self,
        uri: &str,
        header: &str,
        value: &str,
    ) -> (StatusCode, Vec<SseEvent>) {
        let response = self
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .header(header, value)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = collect_sse_body(response.into_body()).await;
        let events = parse_sse_events(&body);
        (status, events)
    }

    async fn get_error(&self, uri: &str) -> (StatusCode, openapi::ErrorResponse) {
        let response = self
            .app
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 16384)
            .await
            .unwrap();
        let err: openapi::ErrorResponse = serde_json::from_slice(&bytes).unwrap();
        (status, err)
    }
}

fn make_envelope() -> Envelope {
    Envelope::sync_completed("dev-1", Utc::now())
}

// ============================= SSE body helpers ============================== //

#[derive(Debug)]
struct SseEvent {
    id: Option<String>,
    event: Option<String>,
    data: Option<String>,
}

async fn collect_sse_body(mut body: Body) -> Vec<u8> {
    let mut buf = Vec::new();
    loop {
        match tokio::time::timeout(Duration::from_millis(100), body.frame()).await {
            Ok(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    buf.extend_from_slice(data);
                }
            }
            _ => break,
        }
    }
    buf
}

fn parse_sse_events(body: &[u8]) -> Vec<SseEvent> {
    let text = String::from_utf8_lossy(body);
    let mut events = Vec::new();

    // SSE events are separated by double newlines
    for block in text.split("\n\n") {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }

        let mut id = None;
        let mut event = None;
        let mut data = None;

        for line in block.lines() {
            if let Some(rest) = line.strip_prefix("id:") {
                id = Some(rest.trim().to_string());
            } else if let Some(rest) = line.strip_prefix("event:") {
                event = Some(rest.trim().to_string());
            } else if let Some(rest) = line.strip_prefix("data:") {
                data = Some(rest.trim().to_string());
            }
        }

        // Only include blocks that have at least one SSE field (skip heartbeat comments)
        if id.is_some() || event.is_some() || data.is_some() {
            events.push(SseEvent { id, event, data });
        }
    }

    events
}

// ================================= Tests ===================================== //

mod cursor {
    use super::*;

    #[tokio::test]
    async fn no_cursor_returns_all_events() {
        let f = Fixture::new("sse_no_cursor").await;
        f.publish(make_envelope());
        f.publish(make_envelope());

        let (status, events) = f.get_sse("/v0.2/events").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(events.len(), 2);
    }

    #[tokio::test]
    async fn after_query_param() {
        let f = Fixture::new("sse_after_param").await;
        f.publish(make_envelope());
        f.publish(make_envelope());
        f.publish(make_envelope());

        let (status, events) = f.get_sse("/v0.2/events?after=2").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id.as_deref(), Some("3"));
    }

    #[tokio::test]
    async fn last_event_id_header() {
        let f = Fixture::new("sse_last_event_id").await;
        f.publish(make_envelope());
        f.publish(make_envelope());
        f.publish(make_envelope());

        let (status, events) = f
            .get_sse_with_header("/v0.2/events", "Last-Event-ID", "1")
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(events.len(), 2);
    }

    #[tokio::test]
    async fn query_param_takes_precedence() {
        let f = Fixture::new("sse_param_precedence").await;
        f.publish(make_envelope());
        f.publish(make_envelope());
        f.publish(make_envelope());

        let (status, events) = f
            .get_sse_with_header("/v0.2/events?after=2", "Last-Event-ID", "0")
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(events.len(), 1);
    }

    #[tokio::test]
    async fn malformed_cursor_returns_400() {
        let f = Fixture::new("sse_malformed_cursor").await;

        let (status, err) = f.get_error("/v0.2/events?after=abc").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(err.error.code, "malformed_cursor");
    }

    #[tokio::test]
    async fn expired_cursor_returns_410() {
        let f = Fixture::new_with_max_retained("sse_expired_cursor", 2).await;
        for _ in 0..5 {
            f.publish(make_envelope());
        }

        let (status, err) = f.get_error("/v0.2/events?after=1").await;
        assert_eq!(status, StatusCode::GONE);
        assert_eq!(err.error.code, "cursor_expired");
    }

    #[tokio::test]
    async fn one_before_earliest_cursor_is_accepted() {
        let f = Fixture::new_with_max_retained("sse_cursor_before_earliest", 2).await;
        for _ in 0..5 {
            f.publish(make_envelope());
        }

        let (status, events) = f.get_sse("/v0.2/events?after=3").await;
        assert_eq!(status, StatusCode::OK);
        let ids: Vec<&str> = events
            .iter()
            .filter_map(|event| event.id.as_deref())
            .collect();
        assert_eq!(ids, vec!["4", "5"]);
    }
}

mod replay {
    use super::*;

    #[tokio::test]
    async fn events_in_ascending_order() {
        let f = Fixture::new("sse_ascending").await;
        f.publish(make_envelope());
        f.publish(make_envelope());
        f.publish(make_envelope());

        let (_, events) = f.get_sse("/v0.2/events").await;
        let ids: Vec<&str> = events.iter().filter_map(|e| e.id.as_deref()).collect();
        assert_eq!(ids, vec!["1", "2", "3"]);
    }

    #[tokio::test]
    async fn limit_param_respected() {
        let f = Fixture::new("sse_limit").await;
        for _ in 0..5 {
            f.publish(make_envelope());
        }

        let (_, events) = f.get_sse("/v0.2/events?limit=2").await;
        assert_eq!(events.len(), 2);
    }

    #[tokio::test]
    async fn zero_limit_returns_empty_response() {
        let f = Fixture::new("sse_zero_limit").await;
        for _ in 0..3 {
            f.publish(make_envelope());
        }

        let (status, events) = f.get_sse("/v0.2/events?limit=0").await;
        assert_eq!(status, StatusCode::OK);
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn empty_store_returns_empty_response() {
        let f = Fixture::new("sse_empty").await;

        let (status, events) = f.get_sse("/v0.2/events").await;
        assert_eq!(status, StatusCode::OK);
        assert!(events.is_empty());
    }
}

mod format {
    use super::*;

    #[tokio::test]
    async fn sse_event_has_correct_fields() {
        let f = Fixture::new("sse_format").await;
        f.publish(Envelope::sync_completed("dev-1", Utc::now()));

        let (_, events) = f.get_sse("/v0.2/events").await;
        assert_eq!(events.len(), 1);

        let sse = &events[0];
        assert_eq!(sse.id.as_deref(), Some("1"));
        assert_eq!(sse.event.as_deref(), Some("sync.completed"));

        // data should be valid JSON containing the full envelope
        let data_json: serde_json::Value =
            serde_json::from_str(sse.data.as_ref().unwrap()).unwrap();
        assert_eq!(data_json["id"], 1);
        assert_eq!(data_json["type"], "sync.completed");
        assert_eq!(data_json["device_id"], "dev-1");
    }
}
