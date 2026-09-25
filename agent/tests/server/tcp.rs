// standard crates
use std::net::Ipv4Addr;
use std::sync::Arc;

// internal crates
use crate::mocks::http_client::MockClient;
use crate::test_utils::{
    filesys::dirs as test_dirs,
    sync::{create_storage, create_token_manager},
};
use device_api::models::ApiVersion;
use miru_agent::activity;
use miru_agent::events::hub::{EventHub, SpawnOptions};
use miru_agent::server::{tcp, ServerErr, State};
use miru_agent::sync::Syncer;

// external crates
use tokio::sync::{broadcast, mpsc};

struct Fixture {
    state: Arc<State>,
    shutdown_tx: broadcast::Sender<()>,
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
            _dir: dir,
        }
    }
}

fn no_proxy_client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
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
        let probe = tcp::bind(0).await.unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);

        let listener = tcp::bind(port).await.unwrap();
        assert_eq!(listener.local_addr().unwrap().port(), port);
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
        let listener = tcp::bind(0).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut shutdown_rx = fixture.shutdown_tx.subscribe();
        let handle = tcp::serve(listener, fixture.state.clone(), async move {
            let _ = shutdown_rx.recv().await;
        });

        let url = format!("http://{addr}/{}/health", ApiVersion::API_VERSION);
        let response = no_proxy_client().get(&url).send().await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);

        let _ = fixture.shutdown_tx.send(());
        handle.await.unwrap().unwrap();
    }
}
