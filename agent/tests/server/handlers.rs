// internal crates
use device_api::models::{HealthResponse, VersionResponse};
use miru_agent::server::handlers;
use miru_agent::version::{self, COMMIT, VERSION};

// external crates
use axum::body;
use axum::http::StatusCode;
use axum::response::IntoResponse;

pub mod health {
    use super::*;

    #[tokio::test]
    async fn returns_ok_with_status() {
        let response = handlers::health().await.into_response();
        assert_eq!(response.status(), StatusCode::OK);

        let bytes = body::to_bytes(response.into_body(), 1024).await.unwrap();
        let actual: HealthResponse = serde_json::from_slice(&bytes).unwrap();
        let expected = HealthResponse {
            status: "ok".to_string(),
        };
        assert_eq!(actual, expected);
    }
}

pub mod version_tests {
    use super::*;

    #[tokio::test]
    async fn returns_ok_with_version_and_commit() {
        let response = handlers::version().await.into_response();
        assert_eq!(response.status(), StatusCode::OK);

        let bytes = body::to_bytes(response.into_body(), 4096).await.unwrap();
        let actual: VersionResponse = serde_json::from_slice(&bytes).unwrap();
        let expected = VersionResponse {
            version: VERSION.to_string(),
            git_commit: COMMIT.to_string(),
            api_version: version::api_version(),
            api_git_commit: version::api_git_commit(),
            api_release_version: version::api_release_version(),
            rust_version: version::RUST_VERSION.to_string(),
            build_date: version::BUILD_DATE.to_string(),
            os: version::OS.to_string(),
            arch: version::ARCH.to_string(),
        };
        assert_eq!(actual, expected);
    }
}

pub mod routes {
    use std::sync::Arc;
    use std::time::Duration;

    use axum::body::{self, Body};
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    use device_api::models as openapi;
    use miru_agent::calibrations::{Report, ReportMetadata, UploaderOptions};
    use miru_agent::events::hub::{EventHub, SpawnOptions};
    use miru_agent::filesys::Overwrite;
    use miru_agent::models::{
        Deployment, DplActivity, DplErrStatus, DplTarget, FileRule, FileRuleRetention,
        FileRuleUpload, GitCommit, InstanceFormat, Release,
    };
    use miru_agent::server::{routes, State};
    use miru_agent::sync::Syncer;

    use crate::mocks::calibration_sender::MockCalibrationSender;
    use crate::mocks::http_client::{self as mock, MockClient};
    use crate::test_utils::calibrations::uploader;
    use crate::test_utils::filesys::dirs as test_dirs;
    use crate::test_utils::sync::{create_storage, create_token_manager};

    use chrono::{DateTime, TimeZone, Utc};
    use serde_json::json;
    use tokio::sync::{broadcast, mpsc};

    fn fixed_time() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2025, 6, 15, 12, 0, 0).unwrap()
    }

    struct Fixture {
        state: Arc<State>,
        app: Router,
        calibration_sender: Arc<MockCalibrationSender>,
        calibration_started: mpsc::UnboundedReceiver<()>,
        _dir: test_dirs::TempDir,
        _backend: mock::Server,
    }

    impl Fixture {
        async fn new(name: &str) -> Self {
            Self::with_calibration_options(name, UploaderOptions::default()).await
        }

        async fn with_calibration_options(name: &str, options: UploaderOptions) -> Self {
            let dir = test_dirs::temp(name).unwrap();
            let storage = Arc::new(create_storage(dir.dir()).await);
            let http_client = Arc::new(MockClient::default());
            let (token_mngr, _handle) = create_token_manager(dir.dir(), http_client.clone()).await;
            let (sender, _receiver) = mpsc::channel(1);
            let syncer = Arc::new(Syncer::new(sender));

            // Backend mock server. Handlers that reach the cache-miss fallback
            // hit this server; all routes respond with 404 so the raw
            // RequestFailed error propagates through (HTTP 404 to clients).
            let backend_router = Router::new()
                .route("/deployments/{id}", get(mock::not_found))
                .route("/releases/{id}", get(mock::not_found))
                .route("/git_commits/{id}", get(mock::not_found))
                .route("/file_rules/{id}", get(mock::not_found));
            let backend = mock::run_server(backend_router).await;
            let real_http_client =
                Arc::new(miru_agent::http::Client::new(&backend.base_url).unwrap());

            let log_file = dir.file("events.jsonl");
            let (event_hub, _hub_handle) = EventHub::spawn(log_file, SpawnOptions::default())
                .await
                .unwrap();

            let (calibration_sender, calibration_started) = MockCalibrationSender::new();
            let calibration_uploader = uploader(&dir, calibration_sender.clone(), options);

            let (shutdown_tx, _) = broadcast::channel::<()>(1);
            let state = Arc::new(State::new(
                storage,
                real_http_client,
                syncer,
                Arc::new(token_mngr),
                event_hub,
                Arc::new(calibration_uploader),
                shutdown_tx,
            ));

            let app = routes::router(state.clone());

            Self {
                state,
                app,
                calibration_sender,
                calibration_started,
                _dir: dir,
                _backend: backend,
            }
        }

        async fn get(&self, uri: &str) -> (StatusCode, Vec<u8>) {
            let response = self
                .app
                .clone()
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            let status = response.status();
            let bytes = body::to_bytes(response.into_body(), 16384).await.unwrap();
            (status, bytes.to_vec())
        }

        async fn post(&self, uri: &str) -> (StatusCode, Vec<u8>) {
            let response = self
                .app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            let bytes = body::to_bytes(response.into_body(), 16384).await.unwrap();
            (status, bytes.to_vec())
        }

        async fn post_json(&self, uri: &str, body: String) -> (StatusCode, Vec<u8>) {
            let response = self
                .app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(uri)
                        .header("content-type", "application/json")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            let bytes = body::to_bytes(response.into_body(), 16384).await.unwrap();
            (status, bytes.to_vec())
        }

        /// The first calibration report the uploader sends.
        async fn uploaded_calibration(&mut self) -> Report {
            tokio::time::timeout(Duration::from_secs(60), self.calibration_started.recv())
                .await
                .unwrap()
                .unwrap();
            self.calibration_sender.reports().remove(0)
        }
    }

    mod device {
        use super::*;

        #[tokio::test]
        async fn get_device_returns_200() {
            let f = Fixture::new("handler_get_device").await;

            let (status, bytes) = f.get("/v0.2/device").await;
            assert_eq!(status, StatusCode::OK);

            let actual: openapi::Device = serde_json::from_slice(&bytes).unwrap();
            // default device has empty id and offline status
            assert_eq!(actual.status, openapi::DeviceStatus::DEVICE_STATUS_OFFLINE);
        }

        #[tokio::test]
        async fn sync_device_returns_500_when_syncer_channel_closed() {
            let f = Fixture::new("handler_sync_device").await;

            // The syncer's receiver is dropped in the fixture, so sync will fail
            let (status, bytes) = f.post("/v0.2/device/sync").await;
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);

            let actual: openapi::ErrorResponse = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(actual.error.code, "internal_server_error");
        }
    }

    mod deployments {
        use super::*;

        #[tokio::test]
        async fn get_deployment_returns_200() {
            let f = Fixture::new("handler_get_dpl").await;
            let t = fixed_time();
            let dpl = Deployment {
                id: "dpl-1".into(),
                description: "test deploy".into(),
                activity_status: DplActivity::Deployed,
                error_status: DplErrStatus::None,
                target_status: DplTarget::Deployed,
                device_id: "dev-1".into(),
                release_id: "rls-1".into(),
                created_at: t,
                updated_at: t,
                ..Default::default()
            };
            f.state
                .storage
                .deployments
                .write("dpl-1".to_string(), dpl, |_, _| false, Overwrite::Allow)
                .await
                .unwrap();

            let (status, bytes) = f.get("/v0.2/deployments/dpl-1").await;
            assert_eq!(status, StatusCode::OK);

            let actual: openapi::Deployment = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(actual.id, "dpl-1");
            assert_eq!(
                actual.activity_status,
                openapi::DeploymentActivityStatus::DEPLOYMENT_ACTIVITY_STATUS_DEPLOYED
            );
        }

        #[tokio::test]
        async fn get_deployment_returns_404_when_missing() {
            let f = Fixture::new("handler_get_dpl_404").await;

            let (status, bytes) = f.get("/v0.2/deployments/nonexistent").await;
            assert_eq!(status, StatusCode::NOT_FOUND);

            let actual: openapi::ErrorResponse = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(actual.error.code, "internal_server_error");
        }

        #[tokio::test]
        async fn get_current_deployment_returns_200() {
            let f = Fixture::new("handler_get_cur_dpl").await;
            let t = fixed_time();
            let dpl = Deployment {
                id: "dpl-cur".into(),
                description: "current".into(),
                activity_status: DplActivity::Deployed,
                error_status: DplErrStatus::None,
                target_status: DplTarget::Deployed,
                device_id: "dev-1".into(),
                release_id: "rls-1".into(),
                created_at: t,
                updated_at: t,
                ..Default::default()
            };
            f.state
                .storage
                .deployments
                .write("dpl-cur".to_string(), dpl, |_, _| false, Overwrite::Allow)
                .await
                .unwrap();

            let (status, bytes) = f.get("/v0.2/deployments/current").await;
            assert_eq!(status, StatusCode::OK);

            let actual: openapi::Deployment = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(actual.id, "dpl-cur");
        }

        #[tokio::test]
        async fn get_current_deployment_returns_404_when_none_deployed() {
            let f = Fixture::new("handler_get_cur_dpl_404").await;

            // Store a queued deployment (not deployed)
            let dpl = Deployment {
                id: "dpl-queued".into(),
                activity_status: DplActivity::Queued,
                ..Default::default()
            };
            f.state
                .storage
                .deployments
                .write(
                    "dpl-queued".to_string(),
                    dpl,
                    |_, _| false,
                    Overwrite::Allow,
                )
                .await
                .unwrap();

            let (status, _bytes) = f.get("/v0.2/deployments/current").await;
            assert_eq!(status, StatusCode::NOT_FOUND);
        }

        #[tokio::test]
        async fn current_route_not_captured_as_deployment_id() {
            let f = Fixture::new("handler_cur_route").await;
            let t = fixed_time();

            // Store a deployment whose ID is literally "current"
            let dpl = Deployment {
                id: "current".into(),
                description: "named current".into(),
                activity_status: DplActivity::Queued,
                error_status: DplErrStatus::None,
                target_status: DplTarget::Staged,
                device_id: "dev-1".into(),
                release_id: "rls-1".into(),
                created_at: t,
                updated_at: t,
                ..Default::default()
            };
            f.state
                .storage
                .deployments
                .write("current".to_string(), dpl, |_, _| false, Overwrite::Allow)
                .await
                .unwrap();

            // GET /deployments/current should resolve via find_one("deployed")
            // and return 404 (no deployed deployment), NOT the deployment named "current"
            let (status, _) = f.get("/v0.2/deployments/current").await;
            assert_eq!(status, StatusCode::NOT_FOUND);
        }
    }

    mod releases {
        use super::*;

        #[tokio::test]
        async fn get_release_returns_200() {
            let f = Fixture::new("handler_get_rls").await;
            let t = fixed_time();
            let rls = Release {
                id: "rls-1".into(),
                version: "1.0.0".into(),
                git_commit_id: Some("gc-1".into()),
                os: miru_agent::models::Os::Linux,
                created_at: t,
                updated_at: t,
                file_rule_ids: Vec::new(),
            };
            f.state
                .storage
                .releases
                .write("rls-1".to_string(), rls, |_, _| false, Overwrite::Allow)
                .await
                .unwrap();

            let (status, bytes) = f.get("/v0.2/releases/rls-1").await;
            assert_eq!(status, StatusCode::OK);

            let actual: openapi::Release = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(actual.id, "rls-1");
            assert_eq!(actual.version, "1.0.0");
            assert_eq!(actual.git_commit_id, Some("gc-1".into()));
        }

        #[tokio::test]
        async fn get_release_returns_404_when_missing() {
            let f = Fixture::new("handler_get_rls_404").await;

            let (status, bytes) = f.get("/v0.2/releases/nonexistent").await;
            assert_eq!(status, StatusCode::NOT_FOUND);

            let actual: openapi::ErrorResponse = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(actual.error.code, "internal_server_error");
        }

        #[tokio::test]
        async fn get_current_release_returns_200() {
            let f = Fixture::new("handler_get_cur_rls").await;
            let t = fixed_time();

            // Store a deployed deployment referencing rls-1
            let dpl = Deployment {
                id: "dpl-1".into(),
                activity_status: DplActivity::Deployed,
                error_status: DplErrStatus::None,
                target_status: DplTarget::Deployed,
                release_id: "rls-1".into(),
                created_at: t,
                updated_at: t,
                ..Default::default()
            };
            f.state
                .storage
                .deployments
                .write("dpl-1".to_string(), dpl, |_, _| false, Overwrite::Allow)
                .await
                .unwrap();

            // Store the release
            let rls = Release {
                id: "rls-1".into(),
                version: "2.0.0".into(),
                git_commit_id: None,
                os: miru_agent::models::Os::Linux,
                created_at: t,
                updated_at: t,
                file_rule_ids: vec!["fr-1".into(), "fr-2".into()],
            };
            f.state
                .storage
                .releases
                .write("rls-1".to_string(), rls, |_, _| false, Overwrite::Allow)
                .await
                .unwrap();

            let (status, bytes) = f.get("/v0.2/releases/current").await;
            assert_eq!(status, StatusCode::OK);

            let actual: openapi::Release = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(actual.id, "rls-1");
            assert_eq!(actual.version, "2.0.0");
            assert_eq!(actual.file_rule_ids, vec!["fr-1", "fr-2"]);
        }

        #[tokio::test]
        async fn get_current_release_returns_404_when_no_deployed() {
            let f = Fixture::new("handler_get_cur_rls_404").await;

            let (status, _) = f.get("/v0.2/releases/current").await;
            assert_eq!(status, StatusCode::NOT_FOUND);
        }
    }

    mod git_commits {
        use super::*;

        #[tokio::test]
        async fn get_git_commit_returns_200() {
            let f = Fixture::new("handler_get_gc").await;
            let t = fixed_time();
            let gc = GitCommit {
                id: "gc-1".into(),
                sha: "abc123".into(),
                message: "initial commit".into(),
                repository_owner: "owner".into(),
                repository_name: "repo".into(),
                repository_type: "github".into(),
                repository_url: "https://github.com/owner/repo".into(),
                commit_url: "https://github.com/owner/repo/commit/abc123".into(),
                created_at: t,
            };
            f.state
                .storage
                .git_commits
                .write("gc-1".to_string(), gc, |_, _| false, Overwrite::Allow)
                .await
                .unwrap();

            let (status, bytes) = f.get("/v0.2/git_commits/gc-1").await;
            assert_eq!(status, StatusCode::OK);

            let actual: openapi::GitCommit = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(actual.id, "gc-1");
            assert_eq!(actual.sha, "abc123");
            assert_eq!(actual.message, "initial commit");
        }

        #[tokio::test]
        async fn get_git_commit_returns_404_when_missing() {
            let f = Fixture::new("handler_get_gc_404").await;

            let (status, bytes) = f.get("/v0.2/git_commits/nonexistent").await;
            assert_eq!(status, StatusCode::NOT_FOUND);

            let actual: openapi::ErrorResponse = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(actual.error.code, "internal_server_error");
        }
    }

    mod file_rules {
        use super::*;

        fn upload_rule(id: &str) -> FileRule {
            FileRule {
                id: id.into(),
                name: "logs".into(),
                upload: Some(FileRuleUpload {
                    upload_collection_id: "uc-1".into(),
                    upload_collection_name: "logs".into(),
                    bucket_id: "bkt-1".into(),
                    bucket_name: "fleet-logs".into(),
                    path: "robots/".into(),
                    bundle: None,
                }),
                retention: Some(FileRuleRetention {
                    require_upload: Some(true),
                    ttl_secs: 3600,
                }),
                created_at: fixed_time(),
                updated_at: fixed_time(),
                ..Default::default()
            }
        }

        async fn store_rule(f: &Fixture, rule: FileRule) {
            f.state
                .storage
                .file_rules
                .write_if_absent(rule.id.clone(), rule, |_, _| false)
                .await
                .unwrap();
        }

        #[tokio::test]
        async fn get_file_rule_returns_200() {
            let f = Fixture::new("handler_get_fr").await;
            let rule = upload_rule("fr-1");
            store_rule(&f, rule.clone()).await;

            let (status, bytes) = f.get("/v0.2/file_rules/fr-1").await;
            assert_eq!(status, StatusCode::OK);

            let actual: openapi::BaseFileRule = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(actual, openapi::BaseFileRule::from(&rule));
        }

        #[tokio::test]
        async fn get_file_rule_returns_404_when_missing() {
            let f = Fixture::new("handler_get_fr_404").await;

            let (status, bytes) = f.get("/v0.2/file_rules/nonexistent").await;
            assert_eq!(status, StatusCode::NOT_FOUND);

            let actual: openapi::ErrorResponse = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(actual.error.code, "internal_server_error");
        }
    }

    mod calibrations {
        use super::*;

        const URI: &str = "/v0.2/calibrations";

        fn request(format: &str, data: &str) -> String {
            json!({
                "component_slug": "arm",
                "role": "left",
                "aspect": "pose",
                "serial_number": "SN-1",
                "content": { "format": format, "data": data },
            })
            .to_string()
        }

        fn error_code(bytes: &[u8]) -> String {
            let actual: openapi::ErrorResponse = serde_json::from_slice(bytes).unwrap();
            actual.error.code
        }

        #[tokio::test]
        async fn create_calibration_returns_200_and_queues_the_report() {
            let mut f = Fixture::new("handler_create_calibration").await;

            let (status, bytes) = f.post_json(URI, request("yaml", "offset: 1\n")).await;
            assert_eq!(status, StatusCode::OK);

            let actual: openapi::CreateCalibrationResponse =
                serde_json::from_slice(&bytes).unwrap();
            let uploaded = f.uploaded_calibration().await;
            assert_eq!(
                actual.reported_at,
                uploaded.metadata.reported_at.to_rfc3339()
            );
            let expected = Report {
                metadata: ReportMetadata {
                    component: "arm".to_string(),
                    format: InstanceFormat::Yaml,
                    role: Some("left".to_string()),
                    aspect: Some("pose".to_string()),
                    serial_number: Some("SN-1".to_string()),
                    deployment_id: None,
                    reported_at: uploaded.metadata.reported_at,
                },
                data: "offset: 1\n".to_string(),
            };
            assert_eq!(uploaded, expected);
        }

        #[tokio::test]
        async fn create_calibration_accepts_1_mib_of_content_that_escaping_doubles() {
            let mut f = Fixture::new("handler_create_calibration_1mib").await;
            let data = "\n".repeat(1024 * 1024);

            let (status, _) = f.post_json(URI, request("text", &data)).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(f.uploaded_calibration().await.data, data);
        }

        #[tokio::test]
        async fn create_calibration_returns_413_when_content_is_over_1_mib() {
            let f = Fixture::new("handler_create_calibration_too_large").await;
            let data = "x".repeat(1024 * 1024 + 1);

            let (status, bytes) = f.post_json(URI, request("text", &data)).await;
            assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
            assert_eq!(error_code(&bytes), "calibration_too_large");
        }

        #[tokio::test]
        async fn create_calibration_returns_413_when_body_is_over_the_limit() {
            let f = Fixture::new("handler_create_calibration_body_too_large").await;
            let data = "x".repeat(routes::MAX_REQUEST_BYTES);

            let (status, bytes) = f.post_json(URI, request("text", &data)).await;
            assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
            assert_eq!(error_code(&bytes), "calibration_too_large");
        }

        #[tokio::test]
        async fn create_calibration_returns_422_for_malformed_json() {
            let f = Fixture::new("handler_create_calibration_malformed").await;

            let (status, bytes) = f.post_json(URI, "{".to_string()).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
            assert_eq!(error_code(&bytes), "invalid_request");
        }

        #[tokio::test]
        async fn create_calibration_returns_422_when_a_required_field_is_missing() {
            let f = Fixture::new("handler_create_calibration_missing_field").await;
            let body = json!({ "content": { "format": "json", "data": "{}" } }).to_string();

            let (status, bytes) = f.post_json(URI, body).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
            assert_eq!(error_code(&bytes), "invalid_request");
        }

        #[tokio::test]
        async fn create_calibration_returns_422_for_an_unknown_format() {
            let f = Fixture::new("handler_create_calibration_unknown_format").await;

            let (status, bytes) = f.post_json(URI, request("toml", "a = 1")).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
            assert_eq!(error_code(&bytes), "invalid_request");
        }

        #[tokio::test]
        async fn create_calibration_returns_507_when_the_buffer_is_full() {
            let options = UploaderOptions {
                capacity: 0,
                ..UploaderOptions::default()
            };
            let f = Fixture::with_calibration_options("handler_calibration_full", options).await;

            let (status, bytes) = f.post_json(URI, request("json", "{}")).await;
            assert_eq!(status, StatusCode::INSUFFICIENT_STORAGE);
            assert_eq!(error_code(&bytes), "calibration_buffer_full");
        }
    }
}
