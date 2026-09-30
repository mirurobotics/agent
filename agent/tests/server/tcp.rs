// standard crates
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// internal crates
use crate::logs::CapturingWriter;
use crate::mocks::http_client::MockClient;
use crate::test_utils::{
    filesys::dirs as test_dirs,
    sync::{create_storage, create_token_manager},
};
use device_api::models::ApiVersion;
use miru_agent::activity;
use miru_agent::events::hub::{EventHub, SpawnOptions};
use miru_agent::filesys::{self, files, PathExt};
use miru_agent::server::{auth::BearerToken, discovery, routes, tcp, ServerErr, State};
use miru_agent::sync::Syncer;

// external crates
use axum::body::Body;
use axum::http::{header::AUTHORIZATION, Request, StatusCode};
use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tower::ServiceExt;

struct Fixture {
    state: Arc<State>,
    shutdown_tx: broadcast::Sender<()>,
    discovery_file: filesys::File,
    _dir: test_dirs::TempDir,
}

impl Fixture {
    async fn new(name: &str) -> Self {
        let dir = test_dirs::temp(name).unwrap();
        let storage = Arc::new(create_storage(dir.dir()).await);
        let mock_client = Arc::new(MockClient::default());
        let (token_mngr, _handle) = create_token_manager(dir.dir(), mock_client).await;
        let (sender, _receiver) = mpsc::channel(1);
        let syncer = Arc::new(Syncer::new(sender));
        let http_client = Arc::new(miru_agent::http::Client::new("http://localhost:1").unwrap());
        let log_file = dir.file("events.jsonl");
        let (event_hub, _handle) = EventHub::spawn(log_file, SpawnOptions::default())
            .await
            .unwrap();
        let (shutdown_tx, _) = broadcast::channel::<()>(1);
        let discovery_file = dir.dir().subdir("device-api").file("device-api.json");

        let state = Arc::new(State::new(
            storage,
            http_client,
            syncer,
            Arc::new(token_mngr),
            Arc::new(activity::Tracker::new()),
            event_hub,
            shutdown_tx.clone(),
        ));
        Self {
            state,
            shutdown_tx,
            discovery_file,
            _dir: dir,
        }
    }

    async fn token(&self) -> String {
        let discovery: serde_json::Value = files::read_json(&self.discovery_file).await.unwrap();
        discovery["token"].as_str().unwrap().to_string()
    }
}

fn no_proxy_client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}

/// Bind an OS-assigned loopback port and serve until `stop`.
async fn start(fixture: &Fixture) -> (SocketAddr, JoinHandle<Result<(), ServerErr>>) {
    let listener = tcp::bind(0).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mut shutdown_rx = fixture.shutdown_tx.subscribe();
    let handle = tcp::serve(
        listener,
        fixture.state.clone(),
        fixture.discovery_file.clone(),
        async move {
            let _ = shutdown_rx.recv().await;
        },
    )
    .await
    .unwrap();
    (addr, handle)
}

/// Signal shutdown and assert the server exits cleanly within 10s.
async fn stop(fixture: &Fixture, handle: JoinHandle<Result<(), ServerErr>>) {
    let _ = fixture.shutdown_tx.send(());
    let joined = tokio::time::timeout(Duration::from_secs(10), handle)
        .await
        .expect("server did not shut down within 10s");
    joined.expect("server task panicked").unwrap();
}

fn health_url(addr: SocketAddr) -> String {
    format!("http://{addr}/{}/health", ApiVersion::API_VERSION)
}

pub mod bind {
    use super::*;

    #[tokio::test]
    async fn binds_ipv4_loopback_with_os_assigned_port() {
        let listener = tcp::bind(0).await.unwrap();
        let addr = listener.local_addr().unwrap();
        assert_eq!(addr.ip(), Ipv4Addr::LOCALHOST);
        assert_ne!(addr.port(), 0);
    }

    #[tokio::test]
    async fn binds_requested_port() {
        // Another parallel test can bind the released port before we do.
        for _ in 0..8 {
            let probe = tcp::bind(0).await.unwrap();
            let port = probe.local_addr().unwrap().port();
            drop(probe);

            let Ok(listener) = tcp::bind(port).await else {
                continue;
            };
            assert_eq!(listener.local_addr().unwrap().port(), port);
            return;
        }
        panic!("could not rebind a port this process just released");
    }

    #[tokio::test]
    async fn errors_when_port_in_use() {
        let taken = tcp::bind(0).await.unwrap();
        let port = taken.local_addr().unwrap().port();

        let err = tcp::bind(port).await.expect_err("port is already bound");
        assert!(matches!(err, ServerErr::BindTcpListenerErr(_)));
        assert!(err.to_string().contains(&format!("127.0.0.1:{port}")));
    }
}

pub mod serve {
    use super::*;

    #[tokio::test]
    async fn serves_routes_over_loopback() {
        let fixture = Fixture::new("tcp_serves_routes_over_loopback").await;
        let (addr, handle) = start(&fixture).await;

        let response = no_proxy_client()
            .get(health_url(addr))
            .bearer_auth(fixture.token().await)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);

        stop(&fixture, handle).await;
    }

    #[tokio::test]
    async fn rejects_foreign_host() {
        let fixture = Fixture::new("tcp_rejects_foreign_host").await;
        let (addr, handle) = start(&fixture).await;

        let response = no_proxy_client()
            .get(health_url(addr))
            .header(
                reqwest::header::HOST,
                format!("attacker.example:{}", addr.port()),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);

        stop(&fixture, handle).await;
    }

    #[tokio::test]
    async fn rejects_missing_token() {
        let fixture = Fixture::new("tcp_rejects_missing_token").await;
        let (addr, handle) = start(&fixture).await;

        let response = no_proxy_client()
            .get(health_url(addr))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
        assert_eq!(
            response
                .headers()
                .get(reqwest::header::WWW_AUTHENTICATE)
                .unwrap(),
            "Bearer"
        );

        stop(&fixture, handle).await;
    }

    #[tokio::test]
    async fn rejects_wrong_token() {
        let fixture = Fixture::new("tcp_rejects_wrong_token").await;
        let (addr, handle) = start(&fixture).await;

        let response = no_proxy_client()
            .get(health_url(addr))
            .bearer_auth(BearerToken::generate().unwrap().expose())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);

        stop(&fixture, handle).await;
    }

    #[tokio::test]
    async fn foreign_host_with_valid_token_is_403() {
        let fixture = Fixture::new("tcp_foreign_host_with_valid_token").await;
        let (addr, handle) = start(&fixture).await;

        let response = no_proxy_client()
            .get(health_url(addr))
            .bearer_auth(fixture.token().await)
            .header(
                reqwest::header::HOST,
                format!("attacker.example:{}", addr.port()),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);

        stop(&fixture, handle).await;
    }

    #[tokio::test]
    async fn sse_streams_heartbeat_with_token() {
        let fixture = Fixture::new("tcp_sse_streams_heartbeat_with_token").await;
        let (addr, handle) = start(&fixture).await;

        let url = format!("http://{addr}/{}/events", ApiVersion::API_VERSION);
        let mut response = no_proxy_client()
            .get(url)
            .bearer_auth(fixture.token().await)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(
            content_type.starts_with("text/event-stream"),
            "unexpected content type: {content_type}"
        );

        let chunk = tokio::time::timeout(Duration::from_secs(5), response.chunk())
            .await
            .expect("no SSE bytes within 5s")
            .unwrap()
            .expect("SSE stream ended before the heartbeat");
        let text = String::from_utf8_lossy(&chunk);
        assert!(text.contains("heartbeat"), "unexpected first chunk: {text}");

        // closing the stream lets graceful shutdown finish
        drop(response);
        stop(&fixture, handle).await;
    }

    #[tokio::test]
    async fn sse_without_token_is_401() {
        let fixture = Fixture::new("tcp_sse_without_token_is_401").await;
        let (addr, handle) = start(&fixture).await;

        let url = format!("http://{addr}/{}/events", ApiVersion::API_VERSION);
        let response = no_proxy_client().get(url).send().await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);

        stop(&fixture, handle).await;
    }

    #[tokio::test]
    async fn discovery_file_matches_listener() {
        let fixture = Fixture::new("tcp_discovery_file_matches_listener").await;
        let (addr, handle) = start(&fixture).await;

        let value: Value = files::read_json(&fixture.discovery_file).await.unwrap();
        let token = value["token"].as_str().unwrap();
        assert_eq!(token.len(), 43);
        assert_eq!(
            value,
            json!({
                "schema_version": discovery::SCHEMA_VERSION,
                "port": addr.port(),
                "token": token,
            })
        );

        stop(&fixture, handle).await;
    }

    #[tokio::test]
    async fn restart_rotates_token() {
        let fixture = Fixture::new("tcp_restart_rotates_token").await;

        let (_, handle) = start(&fixture).await;
        let first = fixture.token().await;
        stop(&fixture, handle).await;

        let (_, handle) = start(&fixture).await;
        let second = fixture.token().await;
        stop(&fixture, handle).await;

        assert_ne!(first, second);
    }

    #[tokio::test]
    async fn discovery_file_removed_after_shutdown() {
        let fixture = Fixture::new("tcp_discovery_file_removed_after_shutdown").await;
        let (_, handle) = start(&fixture).await;
        assert!(fixture.discovery_file.exists());

        stop(&fixture, handle).await;
        assert!(!fixture.discovery_file.exists());
    }

    #[tokio::test]
    async fn discovery_remove_failure_keeps_serve_result() {
        let fixture = Fixture::new("tcp_discovery_remove_failure").await;
        let (_, handle) = start(&fixture).await;

        // a directory at the discovery path makes the removal fail
        let path = fixture.discovery_file.path();
        std::fs::remove_file(path).unwrap();
        std::fs::create_dir(path).unwrap();

        // the removal failure is logged, not returned
        stop(&fixture, handle).await;
        assert!(path.is_dir());
    }

    #[tokio::test]
    async fn discovery_write_failure_errors() {
        let fixture = Fixture::new("tcp_discovery_write_failure_errors").await;
        // a file at the discovery directory path makes the write fail
        let parent = fixture.discovery_file.parent().unwrap();
        std::fs::write(parent.path(), b"not a dir").unwrap();

        let listener = tcp::bind(0).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let result = tcp::serve(
            listener,
            fixture.state.clone(),
            fixture.discovery_file.clone(),
            async {},
        )
        .await;
        let Err(e) = result else {
            panic!("serve started despite the discovery write failure");
        };
        assert!(
            matches!(e, ServerErr::FileSysErr(_)),
            "unexpected error: {e:?}"
        );

        // the listener is dropped, so connections are refused
        let connect = tokio::time::timeout(
            Duration::from_secs(10),
            tokio::net::TcpStream::connect(addr),
        )
        .await
        .expect("connect did not resolve within 10s");
        assert!(connect.is_err(), "listener still accepts connections");
    }
}

pub mod redaction {
    use super::*;

    #[tokio::test]
    async fn authorization_header_is_redacted_in_trace_spans() {
        let fixture = Fixture::new("tcp_authorization_header_is_redacted").await;
        let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_ansi(false)
            .with_writer(CapturingWriter(buf.clone()))
            .finish();
        // current-thread runtime, so the thread-local subscriber sees every span
        let _guard = tracing::subscriber::set_default(subscriber);

        let request = Request::get(format!("/{}/health", ApiVersion::API_VERSION))
            .header(AUTHORIZATION, "Bearer not-a-real-secret")
            .header(AUTHORIZATION, "Bearer also-not-a-secret")
            .body(Body::empty())
            .unwrap();
        let response = routes::router(fixture.state.clone())
            .oneshot(request)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let captured = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        assert!(
            captured.contains("Sensitive"),
            "trace output lacks a redacted header: {captured}"
        );
        assert!(
            !captured.contains("not-a-real-secret"),
            "trace output leaks the first credential: {captured}"
        );
        assert!(
            !captured.contains("also-not-a-secret"),
            "trace output leaks the second credential: {captured}"
        );
    }
}
