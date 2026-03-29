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

    use crate::http::mock::MockClient;
    use crate::sync::syncer::{create_storage, create_token_manager};

    use chrono::{DateTime, TimeZone, Utc};
    use tokio::sync::mpsc;

    fn fixed_time() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2025, 6, 15, 12, 0, 0).unwrap()
    }

    struct Fixture {
        state: Arc<State>,
        app: Router,
        _dir: filesys::Dir,
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

            // State expects Arc<http::Client>, but we only need it to exist;
            // the handlers under test don't use it. Use a real client at a dummy URL.
            let real_http_client =
                Arc::new(miru_agent::http::Client::new("http://localhost:1").unwrap());

            let log_file = dir.file("events.jsonl");
            let (event_hub, _hub_handle) =
                EventHub::spawn(log_file, SpawnOptions::default()).unwrap();

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
            assert_eq!(actual.error.code, "resource_not_found");
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
            assert_eq!(actual.error.code, "resource_not_found");
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
            assert_eq!(actual.error.code, "resource_not_found");
        }
    }
}
