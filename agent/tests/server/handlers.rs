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

    use axum::body::{self, Body};
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    use device_api::models as openapi;
    use miru_agent::activity;
    use miru_agent::events::hub::{EventHub, SpawnOptions};
    use miru_agent::filesys::{self, Overwrite};
    use miru_agent::models::{
        Deployment, DplActivity, DplErrStatus, DplTarget, GitCommit, Release,
    };
    use miru_agent::server::{serve, State};
    use miru_agent::sync::Syncer;

    use crate::mocks::http_client::{self as mock, MockClient};
    use crate::sync::syncer::{create_storage, create_token_manager};

    use chrono::{DateTime, TimeZone, Utc};
    use tokio::sync::{broadcast, mpsc};

    fn fixed_time() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2025, 6, 15, 12, 0, 0).unwrap()
    }

    struct Fixture {
        state: Arc<State>,
        app: Router,
        _dir: filesys::Dir,
        _backend: mock::Server,
    }

    impl Fixture {
        async fn new(name: &str) -> Self {
            let dir = filesys::Dir::create_temp_dir(name).await.unwrap();
            let storage = Arc::new(create_storage(&dir).await);
            let http_client = Arc::new(MockClient::default());
            let (token_mngr, _handle) = create_token_manager(&dir, http_client.clone()).await;
            let (sender, _receiver) = mpsc::channel(1);
            let syncer = Arc::new(Syncer::new(sender));
            let activity_tracker = Arc::new(activity::Tracker::new());

            // Backend mock server. Handlers that reach the cache-miss fallback
            // hit this server; all routes respond with 404 so the raw
            // RequestFailed error propagates through (HTTP 404 to clients).
            let backend_router = Router::new()
                .route("/deployments/{id}", get(mock::not_found))
                .route("/releases/{id}", get(mock::not_found))
                .route("/git_commits/{id}", get(mock::not_found));
            let backend = mock::run_server(backend_router).await;
            let real_http_client =
                Arc::new(miru_agent::http::Client::new(&backend.base_url).unwrap());

            let log_file = dir.file("events.jsonl");
            let (event_hub, _hub_handle) = EventHub::spawn(log_file, SpawnOptions::default())
                .await
                .unwrap();

            let (shutdown_tx, _) = broadcast::channel::<()>(1);
            let state = Arc::new(State::new(
                storage,
                real_http_client,
                syncer,
                Arc::new(token_mngr),
                activity_tracker,
                event_hub,
                shutdown_tx,
            ));

            let app = serve::routes(state.clone());

            Self {
                state,
                app,
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
                created_at: t,
                updated_at: t,
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
                created_at: t,
                updated_at: t,
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

    mod config_instances {
        use super::*;
        use miru_agent::models::ConfigInstance;
        use miru_agent::server::responses::config_instance::{
            ConfigInstanceResponse, ParameterListResponse, ParameterResponse,
        };

        fn sample_ci(id: &str, filepath: &str) -> ConfigInstance {
            ConfigInstance {
                id: id.to_string(),
                config_type_name: "robot_config".to_string(),
                filepath: filepath.to_string(),
                created_at: fixed_time(),
                config_schema_id: "schema-1".to_string(),
                config_type_id: "type-1".to_string(),
            }
        }

        #[tokio::test]
        async fn get_config_instance_returns_200() {
            let f = Fixture::new("handler_get_ci").await;
            let ci = sample_ci("ci-1", "config.json");
            f.state
                .storage
                .cfg_insts
                .meta
                .write("ci-1".to_string(), ci, |_, _| false, Overwrite::Allow)
                .await
                .unwrap();

            let (status, bytes) = f.get("/v0.2/config_instances/ci-1").await;
            assert_eq!(status, StatusCode::OK);

            let actual: ConfigInstanceResponse = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(actual.object, "config_instance");
            assert_eq!(actual.id, "ci-1");
            assert!(actual.content.is_none());
        }

        #[tokio::test]
        async fn get_config_instance_with_expand_content() {
            let f = Fixture::new("handler_get_ci_expand").await;
            let ci = sample_ci("ci-1", "config.json");
            f.state
                .storage
                .cfg_insts
                .meta
                .write("ci-1".to_string(), ci, |_, _| false, Overwrite::Allow)
                .await
                .unwrap();
            f.state
                .storage
                .cfg_insts
                .content
                .write(
                    "ci-1".to_string(),
                    r#"{"key":"value"}"#.to_string(),
                    |_, _| false,
                    Overwrite::Allow,
                )
                .await
                .unwrap();

            let (status, bytes) = f.get("/v0.2/config_instances/ci-1?expand=content").await;
            assert_eq!(status, StatusCode::OK);

            let actual: ConfigInstanceResponse = serde_json::from_slice(&bytes).unwrap();
            assert!(actual.content.is_some());
            let content = actual.content.unwrap();
            assert_eq!(content.format, "json");
            assert_eq!(content.data, r#"{"key":"value"}"#);
        }

        #[tokio::test]
        async fn get_config_instance_returns_404_when_missing() {
            let f = Fixture::new("handler_get_ci_404").await;

            let (status, bytes) = f.get("/v0.2/config_instances/nonexistent").await;
            assert_eq!(status, StatusCode::NOT_FOUND);

            let actual: openapi::ErrorResponse = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(actual.error.code, "resource_not_found");
        }

        #[tokio::test]
        async fn get_content_returns_200_with_json() {
            let f = Fixture::new("handler_get_ci_content_json").await;
            let ci = sample_ci("ci-1", "config.json");
            f.state
                .storage
                .cfg_insts
                .meta
                .write("ci-1".to_string(), ci, |_, _| false, Overwrite::Allow)
                .await
                .unwrap();
            f.state
                .storage
                .cfg_insts
                .content
                .write(
                    "ci-1".to_string(),
                    r#"{"hello":"world"}"#.to_string(),
                    |_, _| false,
                    Overwrite::Allow,
                )
                .await
                .unwrap();

            let response = f
                .app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/v0.2/config_instances/ci-1/content")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers().get("content-type").unwrap(),
                "application/json"
            );
            assert!(response
                .headers()
                .get("content-disposition")
                .unwrap()
                .to_str()
                .unwrap()
                .contains("config.json"));

            let bytes = body::to_bytes(response.into_body(), 16384).await.unwrap();
            assert_eq!(bytes.as_ref(), b"{\"hello\":\"world\"}");
        }

        #[tokio::test]
        async fn get_content_returns_200_with_yaml() {
            let f = Fixture::new("handler_get_ci_content_yaml").await;
            let ci = sample_ci("ci-1", "/etc/miru/robot.yaml");
            f.state
                .storage
                .cfg_insts
                .meta
                .write("ci-1".to_string(), ci, |_, _| false, Overwrite::Allow)
                .await
                .unwrap();
            f.state
                .storage
                .cfg_insts
                .content
                .write(
                    "ci-1".to_string(),
                    "key: value\n".to_string(),
                    |_, _| false,
                    Overwrite::Allow,
                )
                .await
                .unwrap();

            let response = f
                .app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/v0.2/config_instances/ci-1/content")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers().get("content-type").unwrap(),
                "application/x-yaml"
            );
            assert!(response
                .headers()
                .get("content-disposition")
                .unwrap()
                .to_str()
                .unwrap()
                .contains("robot.yaml"));
        }

        #[tokio::test]
        async fn get_content_returns_404_when_missing() {
            let f = Fixture::new("handler_get_ci_content_404").await;

            let (status, bytes) = f.get("/v0.2/config_instances/nonexistent/content").await;
            assert_eq!(status, StatusCode::NOT_FOUND);

            let actual: openapi::ErrorResponse = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(actual.error.code, "resource_not_found");
        }

        #[tokio::test]
        async fn get_parameter_returns_200() {
            let f = Fixture::new("handler_get_ci_param").await;
            let ci = sample_ci("ci-1", "config.json");
            f.state
                .storage
                .cfg_insts
                .meta
                .write("ci-1".to_string(), ci, |_, _| false, Overwrite::Allow)
                .await
                .unwrap();
            f.state
                .storage
                .cfg_insts
                .content
                .write(
                    "ci-1".to_string(),
                    r#"{"network":{"timeout_ms":5000}}"#.to_string(),
                    |_, _| false,
                    Overwrite::Allow,
                )
                .await
                .unwrap();

            let (status, bytes) = f
                .get("/v0.2/config_instances/ci-1/parameters/network.timeout_ms")
                .await;
            assert_eq!(status, StatusCode::OK);

            let actual: ParameterResponse = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(actual.object, "parameter");
            assert_eq!(actual.key, vec!["network", "timeout_ms"]);
            assert_eq!(actual.value, serde_json::json!(5000));
        }

        #[tokio::test]
        async fn get_parameter_returns_404_when_key_missing() {
            let f = Fixture::new("handler_get_ci_param_404").await;
            let ci = sample_ci("ci-1", "config.json");
            f.state
                .storage
                .cfg_insts
                .meta
                .write("ci-1".to_string(), ci, |_, _| false, Overwrite::Allow)
                .await
                .unwrap();
            f.state
                .storage
                .cfg_insts
                .content
                .write(
                    "ci-1".to_string(),
                    r#"{"a":1}"#.to_string(),
                    |_, _| false,
                    Overwrite::Allow,
                )
                .await
                .unwrap();

            let (status, bytes) = f
                .get("/v0.2/config_instances/ci-1/parameters/nonexistent")
                .await;
            assert_eq!(status, StatusCode::NOT_FOUND);

            let actual: openapi::ErrorResponse = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(actual.error.code, "resource_not_found");
        }

        #[tokio::test]
        async fn list_parameters_returns_200() {
            let f = Fixture::new("handler_list_ci_params").await;
            let ci = sample_ci("ci-1", "config.json");
            f.state
                .storage
                .cfg_insts
                .meta
                .write("ci-1".to_string(), ci, |_, _| false, Overwrite::Allow)
                .await
                .unwrap();
            f.state
                .storage
                .cfg_insts
                .content
                .write(
                    "ci-1".to_string(),
                    r#"{"a":1,"b":{"c":2}}"#.to_string(),
                    |_, _| false,
                    Overwrite::Allow,
                )
                .await
                .unwrap();

            let (status, bytes) = f.get("/v0.2/config_instances/ci-1/parameters").await;
            assert_eq!(status, StatusCode::OK);

            let actual: ParameterListResponse = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(actual.object, "list");
            assert_eq!(actual.data.len(), 2);
        }

        #[tokio::test]
        async fn list_parameters_with_prefix_filter() {
            let f = Fixture::new("handler_list_ci_params_prefix").await;
            let ci = sample_ci("ci-1", "config.json");
            f.state
                .storage
                .cfg_insts
                .meta
                .write("ci-1".to_string(), ci, |_, _| false, Overwrite::Allow)
                .await
                .unwrap();
            f.state
                .storage
                .cfg_insts
                .content
                .write(
                    "ci-1".to_string(),
                    r#"{"a":1,"b":{"c":2,"d":3}}"#.to_string(),
                    |_, _| false,
                    Overwrite::Allow,
                )
                .await
                .unwrap();

            let (status, bytes) = f
                .get("/v0.2/config_instances/ci-1/parameters?prefix=b")
                .await;
            assert_eq!(status, StatusCode::OK);

            let actual: ParameterListResponse = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(actual.data.len(), 2);
            for p in &actual.data {
                assert_eq!(p.key[0], "b");
            }
        }
    }
}
