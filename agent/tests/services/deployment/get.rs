// standard crates
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

// internal crates
use backend_api::models as backend_client;
use miru_agent::authn::errors::{AuthnErr, MockError as AuthnMockError};
use miru_agent::cache::errors::CacheErr;
use miru_agent::filesys::{self, Overwrite};
use miru_agent::http::errors::{HTTPErr, MockErr as HttpMockErr, RequestFailed};
use miru_agent::http::request::Params as HttpParams;
use miru_agent::models::{Deployment, DplActivity, DplErrStatus, DplTarget};
use miru_agent::services::deployment::{self as dpl_svc, DeploymentFetcher};
use miru_agent::services::ServiceErr;
use miru_agent::storage::{Deployments, GitCommits, Releases};
use miru_agent::sync::errors::MockErr as SyncMockErr;
use miru_agent::sync::SyncErr;

// external crates
use chrono::{DateTime, Utc};

struct StubDeploymentFetcher {
    result: Mutex<Option<Result<backend_client::Deployment, ServiceErr>>>,
    call_count: AtomicUsize,
}

impl StubDeploymentFetcher {
    fn ok(dpl: backend_client::Deployment) -> Self {
        Self {
            result: Mutex::new(Some(Ok(dpl))),
            call_count: AtomicUsize::new(0),
        }
    }
    fn err(e: ServiceErr) -> Self {
        Self {
            result: Mutex::new(Some(Err(e))),
            call_count: AtomicUsize::new(0),
        }
    }
    fn calls(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }
}

impl DeploymentFetcher for StubDeploymentFetcher {
    async fn fetch_deployment(&self, _id: &str) -> Result<backend_client::Deployment, ServiceErr> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        self.result
            .lock()
            .unwrap()
            .take()
            .expect("stub called more times than canned results provided")
    }
}

struct PanicDeploymentFetcher;
impl DeploymentFetcher for PanicDeploymentFetcher {
    async fn fetch_deployment(&self, _id: &str) -> Result<backend_client::Deployment, ServiceErr> {
        panic!("backend should not be called on cache hit")
    }
}

async fn setup(name: &str) -> (filesys::Dir, Deployments, Releases, GitCommits) {
    let dir = filesys::Dir::create_temp_dir(name).await.unwrap();
    let (dpl_stor, _) = Deployments::spawn(16, dir.file("deployments.json"), 1000)
        .await
        .unwrap();
    let (rls_stor, _) = Releases::spawn(16, dir.file("releases.json"), 1000)
        .await
        .unwrap();
    let (gc_stor, _) = GitCommits::spawn(16, dir.file("git_commits.json"), 1000)
        .await
        .unwrap();
    (dir, dpl_stor, rls_stor, gc_stor)
}

fn make_deployment(id: &str, activity: DplActivity) -> Deployment {
    Deployment {
        id: id.to_string(),
        activity_status: activity,
        error_status: DplErrStatus::None,
        target_status: DplTarget::Deployed,
        created_at: DateTime::<Utc>::UNIX_EPOCH,
        ..Default::default()
    }
}

pub mod get_deployment {
    use super::*;

    #[tokio::test]
    async fn returns_deployment_by_id() {
        let (_dir, dpl_stor, rls_stor, gc_stor) = setup("get_dpl_by_id").await;
        let dpl = make_deployment("dpl_1", DplActivity::Deployed);
        dpl_stor
            .write(
                "dpl_1".to_string(),
                dpl.clone(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();

        let result = dpl_svc::get(
            &dpl_stor,
            &rls_stor,
            &gc_stor,
            None::<&PanicDeploymentFetcher>,
            "dpl_1".to_string(),
        )
        .await
        .unwrap();
        assert_eq!(result.id, "dpl_1");
        assert_eq!(result.activity_status, DplActivity::Deployed);
    }

    #[tokio::test]
    async fn not_found_returns_error() {
        let (_dir, dpl_stor, rls_stor, gc_stor) = setup("get_dpl_not_found").await;

        let result = dpl_svc::get(
            &dpl_stor,
            &rls_stor,
            &gc_stor,
            None::<&PanicDeploymentFetcher>,
            "nonexistent".to_string(),
        )
        .await;
        assert!(matches!(result, Err(ServiceErr::CacheErr(_))));
    }
}

pub mod get_current_deployment {
    use super::*;

    #[tokio::test]
    async fn returns_deployed_deployment() {
        let (_dir, stor, _, _) = setup("get_cur_dpl").await;
        let dpl = make_deployment("dpl_1", DplActivity::Deployed);
        stor.write(
            "dpl_1".to_string(),
            dpl.clone(),
            |_, _| false,
            Overwrite::Allow,
        )
        .await
        .unwrap();

        let result = dpl_svc::get_current(&stor).await.unwrap();
        assert_eq!(result.id, "dpl_1");
        assert_eq!(result.activity_status, DplActivity::Deployed);
    }

    #[tokio::test]
    async fn skips_non_deployed() {
        let (_dir, stor, _, _) = setup("get_cur_dpl_skip").await;
        let queued = make_deployment("dpl_q", DplActivity::Queued);
        stor.write("dpl_q".to_string(), queued, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();
        let deployed = make_deployment("dpl_d", DplActivity::Deployed);
        stor.write(
            "dpl_d".to_string(),
            deployed,
            |_, _| false,
            Overwrite::Allow,
        )
        .await
        .unwrap();

        let result = dpl_svc::get_current(&stor).await.unwrap();
        assert_eq!(result.id, "dpl_d");
    }

    #[tokio::test]
    async fn no_deployed_returns_error() {
        let (_dir, stor, _, _) = setup("get_cur_dpl_none").await;
        let queued = make_deployment("dpl_q", DplActivity::Queued);
        stor.write("dpl_q".to_string(), queued, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();

        let result = dpl_svc::get_current(&stor).await;
        assert!(matches!(result, Err(ServiceErr::CacheErr(_))));
    }

    #[tokio::test]
    async fn empty_cache_returns_error() {
        let (_dir, stor, _, _) = setup("get_cur_dpl_empty").await;

        let result = dpl_svc::get_current(&stor).await;
        assert!(matches!(result, Err(ServiceErr::CacheErr(_))));
    }

    #[tokio::test]
    async fn multiple_deployed_returns_error() {
        let (_dir, stor, _, _) = setup("get_cur_dpl_multi").await;
        let dpl_a = make_deployment("dpl_a", DplActivity::Deployed);
        stor.write("dpl_a".to_string(), dpl_a, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();
        let dpl_b = make_deployment("dpl_b", DplActivity::Deployed);
        stor.write("dpl_b".to_string(), dpl_b, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();

        let result = dpl_svc::get_current(&stor).await;
        assert!(matches!(result, Err(ServiceErr::CacheErr(_))));
    }
}

pub mod get_deployment_fallback {
    use super::*;

    #[tokio::test]
    async fn cache_hit_no_backend_call() {
        let (_dir, dpl_stor, rls_stor, gc_stor) = setup("fb_dpl_cache_hit").await;
        let dpl = make_deployment("dpl_1", DplActivity::Deployed);
        dpl_stor
            .write(
                "dpl_1".to_string(),
                dpl.clone(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();

        let result = dpl_svc::get(
            &dpl_stor,
            &rls_stor,
            &gc_stor,
            Some(&PanicDeploymentFetcher),
            "dpl_1".to_string(),
        )
        .await
        .unwrap();
        assert_eq!(result.id, "dpl_1");
    }

    #[tokio::test]
    async fn cache_miss_backend_hit_caches_value() {
        let (_dir, dpl_stor, rls_stor, gc_stor) = setup("fb_dpl_backend_hit").await;
        let backend_dpl = backend_client::Deployment {
            id: "dpl_1".to_string(),
            description: "test".to_string(),
            device_id: "dvc_1".to_string(),
            release_id: "rls_1".to_string(),
            config_instances: Some(vec![backend_client::ConfigInstance {
                id: "cfg_1".to_string(),
                ..Default::default()
            }]),
            ..Default::default()
        };
        let stub = StubDeploymentFetcher::ok(backend_dpl);

        let result = dpl_svc::get(
            &dpl_stor,
            &rls_stor,
            &gc_stor,
            Some(&stub),
            "dpl_1".to_string(),
        )
        .await
        .unwrap();
        assert_eq!(result.id, "dpl_1");
        assert_eq!(result.device_id, "dvc_1");
        assert_eq!(result.release_id, "rls_1");
        assert_eq!(result.config_instance_ids, vec!["cfg_1".to_string()]);
        assert_eq!(stub.calls(), 1);

        // Second call with PanicDeploymentFetcher must succeed (proves cache).
        let result2 = dpl_svc::get(
            &dpl_stor,
            &rls_stor,
            &gc_stor,
            Some(&PanicDeploymentFetcher),
            "dpl_1".to_string(),
        )
        .await
        .unwrap();
        assert_eq!(result2.id, "dpl_1");
    }

    #[tokio::test]
    async fn cache_miss_backend_hit_caches_expanded_release_and_git_commit() {
        let (_dir, dpl_stor, rls_stor, gc_stor) = setup("fb_dpl_expanded").await;
        let backend_dpl = backend_client::Deployment {
            id: "dpl_1".to_string(),
            description: "test".to_string(),
            device_id: "dvc_1".to_string(),
            release_id: "rls_exp".to_string(),
            release: Some(Box::new(backend_client::Release {
                id: "rls_exp".to_string(),
                version: "1.0.0".to_string(),
                git_commit_id: Some("gc_exp".to_string()),
                git_commit: Some(Some(Box::new(backend_client::GitCommit {
                    id: "gc_exp".to_string(),
                    sha: "deadbeef".to_string(),
                    message: "expanded".to_string(),
                    ..Default::default()
                }))),
                ..Default::default()
            })),
            config_instances: Some(vec![]),
            ..Default::default()
        };
        let stub = StubDeploymentFetcher::ok(backend_dpl);

        let _ = dpl_svc::get(
            &dpl_stor,
            &rls_stor,
            &gc_stor,
            Some(&stub),
            "dpl_1".to_string(),
        )
        .await
        .unwrap();

        assert!(rls_stor
            .read_optional("rls_exp".to_string())
            .await
            .unwrap()
            .is_some());
        assert!(gc_stor
            .read_optional("gc_exp".to_string())
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn cache_miss_backend_404_returns_not_found() {
        let (_dir, dpl_stor, rls_stor, gc_stor) = setup("fb_dpl_404").await;
        let err = ServiceErr::HTTPErr(HTTPErr::RequestFailed(RequestFailed {
            request: HttpParams::get("http://test/cache-miss").meta().unwrap(),
            status: reqwest::StatusCode::NOT_FOUND,
            error: None,
            trace: miru_agent::trace!(),
        }));
        let stub = StubDeploymentFetcher::err(err);

        let result = dpl_svc::get(
            &dpl_stor,
            &rls_stor,
            &gc_stor,
            Some(&stub),
            "dpl_1".to_string(),
        )
        .await;
        assert!(matches!(
            result,
            Err(ServiceErr::CacheErr(CacheErr::CacheElementNotFound(_)))
        ));
    }

    #[tokio::test]
    async fn cache_miss_backend_500_returns_error() {
        let (_dir, dpl_stor, rls_stor, gc_stor) = setup("fb_dpl_500").await;
        let err = ServiceErr::HTTPErr(HTTPErr::RequestFailed(RequestFailed {
            request: HttpParams::get("http://test/cache-miss").meta().unwrap(),
            status: reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            error: None,
            trace: miru_agent::trace!(),
        }));
        let stub = StubDeploymentFetcher::err(err);

        let result = dpl_svc::get(
            &dpl_stor,
            &rls_stor,
            &gc_stor,
            Some(&stub),
            "dpl_1".to_string(),
        )
        .await;
        assert!(matches!(
            result,
            Err(ServiceErr::HTTPErr(HTTPErr::RequestFailed(_)))
        ));
    }

    #[tokio::test]
    async fn cache_miss_backend_network_err_returns_error() {
        let (_dir, dpl_stor, rls_stor, gc_stor) = setup("fb_dpl_network").await;
        let err = ServiceErr::HTTPErr(HTTPErr::MockErr(HttpMockErr {
            is_network_conn_err: true,
        }));
        let stub = StubDeploymentFetcher::err(err);

        let result = dpl_svc::get(
            &dpl_stor,
            &rls_stor,
            &gc_stor,
            Some(&stub),
            "dpl_1".to_string(),
        )
        .await;
        assert!(matches!(result, Err(ServiceErr::HTTPErr(_))));
    }

    #[tokio::test]
    async fn cache_miss_token_err_returns_not_found() {
        let (_dir, dpl_stor, rls_stor, gc_stor) = setup("fb_dpl_token").await;
        let err = ServiceErr::SyncErr(SyncErr::AuthnErr(AuthnErr::MockError(AuthnMockError {
            is_network_conn_err: false,
            trace: miru_agent::trace!(),
        })));
        let stub = StubDeploymentFetcher::err(err);

        let result = dpl_svc::get(
            &dpl_stor,
            &rls_stor,
            &gc_stor,
            Some(&stub),
            "dpl_1".to_string(),
        )
        .await;
        assert!(matches!(
            result,
            Err(ServiceErr::CacheErr(CacheErr::CacheElementNotFound(_)))
        ));
    }

    #[tokio::test]
    async fn cache_miss_non_authn_sync_err_propagates() {
        let (_dir, dpl_stor, rls_stor, gc_stor) = setup("fb_dpl_sync_err").await;
        let err = ServiceErr::SyncErr(SyncErr::MockErr(SyncMockErr {
            is_network_conn_err: false,
        }));
        let stub = StubDeploymentFetcher::err(err);

        let result = dpl_svc::get(
            &dpl_stor,
            &rls_stor,
            &gc_stor,
            Some(&stub),
            "dpl_1".to_string(),
        )
        .await;
        assert!(matches!(
            result,
            Err(ServiceErr::SyncErr(SyncErr::MockErr(_)))
        ));
    }

    #[tokio::test]
    async fn cache_miss_no_backend_returns_not_found() {
        let (_dir, dpl_stor, rls_stor, gc_stor) = setup("fb_dpl_no_backend").await;

        let result = dpl_svc::get(
            &dpl_stor,
            &rls_stor,
            &gc_stor,
            None::<&StubDeploymentFetcher>,
            "dpl_1".to_string(),
        )
        .await;
        assert!(matches!(
            result,
            Err(ServiceErr::CacheErr(CacheErr::CacheElementNotFound(_)))
        ));
    }

    #[test]
    fn resolve_dpl_none_cached_returns_new() {
        let new = make_deployment("dpl_1", DplActivity::Queued);
        let merged = dpl_svc::resolve_dpl(new.clone(), None);
        assert_eq!(merged.id, new.id);
        assert_eq!(merged.activity_status, DplActivity::Queued);
    }

    #[test]
    fn resolve_dpl_cached_preserves_local_state_and_takes_new_target() {
        // Simulates a sync race: the cache was populated between the initial
        // `read_optional` miss and the post-fetch re-read. The merge must
        // preserve local fields from `cached` but take `target_status` and
        // `updated_at` from the newly fetched backend deployment.
        let cached = Deployment {
            id: "dpl_1".to_string(),
            activity_status: DplActivity::Deployed,
            error_status: DplErrStatus::None,
            target_status: DplTarget::Staged,
            updated_at: DateTime::<Utc>::UNIX_EPOCH,
            ..Default::default()
        };
        let new = Deployment {
            id: "dpl_1".to_string(),
            activity_status: DplActivity::Queued,
            error_status: DplErrStatus::None,
            target_status: DplTarget::Deployed,
            updated_at: DateTime::<Utc>::from_timestamp(42, 0).unwrap(),
            ..Default::default()
        };
        let merged = dpl_svc::resolve_dpl(new.clone(), Some(cached.clone()));
        // Local fields preserved from cached.
        assert_eq!(merged.activity_status, DplActivity::Deployed);
        // target_status and updated_at taken from new.
        assert_eq!(merged.target_status, DplTarget::Deployed);
        assert_eq!(merged.updated_at, new.updated_at);
    }

    #[tokio::test]
    async fn cache_miss_backend_missing_config_instances_returns_sync_err() {
        let (_dir, dpl_stor, rls_stor, gc_stor) = setup("fb_dpl_missing_cfg_insts").await;
        // Backend returns a deployment without the expanded config_instances
        // field — the service must surface this as SyncErr::CfgInstsNotExpanded
        // (matching the syncer's contract-violation handling).
        let backend_dpl = backend_client::Deployment {
            id: "dpl_1".to_string(),
            description: "test".to_string(),
            device_id: "dvc_1".to_string(),
            release_id: "rls_1".to_string(),
            config_instances: None,
            ..Default::default()
        };
        let stub = StubDeploymentFetcher::ok(backend_dpl);

        let result = dpl_svc::get(
            &dpl_stor,
            &rls_stor,
            &gc_stor,
            Some(&stub),
            "dpl_1".to_string(),
        )
        .await;
        assert!(matches!(
            result,
            Err(ServiceErr::SyncErr(SyncErr::CfgInstsNotExpanded(_)))
        ));
    }
}
