// standard crates
use std::sync::Arc;

// internal crates
use crate::mocks::{calibration_sender::MockCalibrationSender, http_client::MockClient};
use crate::test_utils::{
    calibrations::{cal_uploader, latest_store},
    filesys::dirs as test_dirs,
    sync::{create_storage, create_token_manager},
};
use miru_daemon::calibrations::{latest, UploaderOptions};
use miru_daemon::events::hub::{EventHub, SpawnOptions};
use miru_daemon::server::{routes, State};
use miru_daemon::sync::Syncer;

// external crates
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tokio::sync::{broadcast, mpsc};
use tower::ServiceExt;

struct Fixture {
    state: Arc<State>,
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
        let http_client = Arc::new(miru_daemon::http::Client::new("http://localhost:1").unwrap());
        let log_file = dir.file("events.jsonl");
        let (event_hub, _handle) = EventHub::spawn(log_file, SpawnOptions::default())
            .await
            .unwrap();
        let latest_cals = latest_store(&dir, latest::DEFAULT_CAPACITY).await;
        let cal_uploader = cal_uploader(
            &dir,
            MockCalibrationSender::new().0,
            UploaderOptions::default(),
            latest_cals.clone(),
        );
        let (shutdown_tx, _) = broadcast::channel::<()>(1);

        let state = Arc::new(State {
            storage,
            http_client,
            syncer,
            token_mngr: Arc::new(token_mngr),
            event_hub,
            cal_uploader: Arc::new(cal_uploader),
            latest_cals,
            shutdown_tx,
        });
        Self { state, _dir: dir }
    }
}

pub mod router {
    use super::*;

    #[tokio::test]
    async fn wrong_method_is_405() {
        let fixture = Fixture::new("routes_wrong_method_is_405").await;
        let request = Request::post("/v0.2/health").body(Body::empty()).unwrap();

        let response = routes::router(fixture.state.clone())
            .oneshot(request)
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    }
}
