// internal crates
use miru_agent::authn::errors::{AuthnErr, MockError as AuthnMockError};
use miru_agent::cache::errors::CacheErr;
use miru_agent::filesys::{self, Overwrite};
use miru_agent::http::errors::{HTTPErr, MockErr as HttpMockErr, RequestFailed};
use miru_agent::http::request::Params as HttpParams;
use miru_agent::models::{Deployment, DplActivity, DplErrStatus, DplTarget, Release};
use miru_agent::services::release::{self as rls_svc, ReleaseFetcher};
use miru_agent::services::ServiceErr;
use miru_agent::storage::{Deployments, Releases};
use miru_agent::sync::errors::MockErr as SyncMockErr;
use miru_agent::sync::SyncErr;

// external crates
use backend_api::models as backend_client;
use chrono::{DateTime, Utc};

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

struct StubReleaseFetcher {
    result: Mutex<Option<Result<backend_client::Release, ServiceErr>>>,
    call_count: AtomicUsize,
}

impl StubReleaseFetcher {
    fn ok(rls: backend_client::Release) -> Self {
        Self {
            result: Mutex::new(Some(Ok(rls))),
            call_count: AtomicUsize::new(0),
        }
    }
    fn err(e: ServiceErr) -> Self {
        Self {
            result: Mutex::new(Some(Err(e))),
            call_count: AtomicUsize::new(0),
        }
    }
    #[allow(dead_code)]
    fn calls(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }
}

impl ReleaseFetcher for StubReleaseFetcher {
    async fn fetch_release(&self, _id: &str) -> Result<backend_client::Release, ServiceErr> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        self.result
            .lock()
            .unwrap()
            .take()
            .expect("stub called more times than canned results provided")
    }
}

struct PanicReleaseFetcher;
impl ReleaseFetcher for PanicReleaseFetcher {
    async fn fetch_release(&self, _id: &str) -> Result<backend_client::Release, ServiceErr> {
        panic!("backend should not be called on cache hit")
    }
}

async fn setup(name: &str) -> (filesys::Dir, Deployments, Releases) {
    let dir = filesys::Dir::create_temp_dir(name).await.unwrap();
    let (dpl_stor, _) = Deployments::spawn(16, dir.file("deployments.json"), 1000)
        .await
        .unwrap();
    let (rls_stor, _) = Releases::spawn(16, dir.file("releases.json"), 1000)
        .await
        .unwrap();
    (dir, dpl_stor, rls_stor)
}

pub mod get_release {
    use super::*;

    #[tokio::test]
    async fn returns_release_by_id() {
        let (_dir, _, rls_stor) = setup("get_rls_by_id").await;
        let rls = Release {
            id: "rls_1".to_string(),
            version: "1.0.0".to_string(),
            ..Default::default()
        };
        rls_stor
            .write(
                "rls_1".to_string(),
                rls.clone(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();

        let result = rls_svc::get(&rls_stor, None::<&PanicReleaseFetcher>, "rls_1".to_string())
            .await
            .unwrap();
        assert_eq!(result.id, "rls_1");
        assert_eq!(result.version, "1.0.0");
    }

    #[tokio::test]
    async fn not_found_returns_error() {
        let (_dir, _, rls_stor) = setup("get_rls_not_found").await;

        let result = rls_svc::get(
            &rls_stor,
            None::<&PanicReleaseFetcher>,
            "nonexistent".to_string(),
        )
        .await;
        assert!(matches!(result, Err(ServiceErr::CacheErr(_))));
    }
}

pub mod get_current_release {
    use super::*;

    #[tokio::test]
    async fn returns_release_for_deployed_deployment() {
        let (_dir, dpl_stor, rls_stor) = setup("get_cur_rls").await;

        let rls = Release {
            id: "rls_1".to_string(),
            version: "2.0.0".to_string(),
            ..Default::default()
        };
        rls_stor
            .write("rls_1".to_string(), rls, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();

        let dpl = Deployment {
            id: "dpl_1".to_string(),
            release_id: "rls_1".to_string(),
            activity_status: DplActivity::Deployed,
            error_status: DplErrStatus::None,
            target_status: DplTarget::Deployed,
            created_at: DateTime::<Utc>::UNIX_EPOCH,
            ..Default::default()
        };
        dpl_stor
            .write("dpl_1".to_string(), dpl, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();

        let result = rls_svc::get_current(&dpl_stor, &rls_stor).await.unwrap();
        assert_eq!(result.id, "rls_1");
        assert_eq!(result.version, "2.0.0");
    }

    #[tokio::test]
    async fn no_deployed_deployment_returns_error() {
        let (_dir, dpl_stor, rls_stor) = setup("get_cur_rls_no_dpl").await;

        let result = rls_svc::get_current(&dpl_stor, &rls_stor).await;
        assert!(matches!(result, Err(ServiceErr::CacheErr(_))));
    }

    #[tokio::test]
    async fn deployed_deployment_with_missing_release_returns_error() {
        let (_dir, dpl_stor, rls_stor) = setup("get_cur_rls_missing").await;

        let dpl = Deployment {
            id: "dpl_1".to_string(),
            release_id: "rls_missing".to_string(),
            activity_status: DplActivity::Deployed,
            error_status: DplErrStatus::None,
            target_status: DplTarget::Deployed,
            created_at: DateTime::<Utc>::UNIX_EPOCH,
            ..Default::default()
        };
        dpl_stor
            .write("dpl_1".to_string(), dpl, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();

        let result = rls_svc::get_current(&dpl_stor, &rls_stor).await;
        assert!(matches!(result, Err(ServiceErr::CacheErr(_))));
    }

    #[tokio::test]
    async fn multiple_deployed_deployments_returns_error() {
        let (_dir, dpl_stor, rls_stor) = setup("get_cur_rls_multi_dpl").await;

        let dpl_a = Deployment {
            id: "dpl_a".to_string(),
            release_id: "rls_1".to_string(),
            activity_status: DplActivity::Deployed,
            error_status: DplErrStatus::None,
            target_status: DplTarget::Deployed,
            created_at: DateTime::<Utc>::UNIX_EPOCH,
            ..Default::default()
        };
        dpl_stor
            .write("dpl_a".to_string(), dpl_a, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();

        let dpl_b = Deployment {
            id: "dpl_b".to_string(),
            release_id: "rls_1".to_string(),
            activity_status: DplActivity::Deployed,
            error_status: DplErrStatus::None,
            target_status: DplTarget::Deployed,
            created_at: DateTime::<Utc>::UNIX_EPOCH,
            ..Default::default()
        };
        dpl_stor
            .write("dpl_b".to_string(), dpl_b, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();

        let result = rls_svc::get_current(&dpl_stor, &rls_stor).await;
        assert!(matches!(result, Err(ServiceErr::CacheErr(_))));
    }
}

pub mod get_release_fallback {
    use super::*;

    #[tokio::test]
    async fn cache_hit_no_backend_call() {
        let (_dir, _, rls_stor) = setup("fb_rls_cache_hit").await;
        let rls = Release {
            id: "rls_1".to_string(),
            version: "1.0.0".to_string(),
            ..Default::default()
        };
        rls_stor
            .write(
                "rls_1".to_string(),
                rls.clone(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();

        let result = rls_svc::get(&rls_stor, Some(&PanicReleaseFetcher), "rls_1".to_string())
            .await
            .unwrap();
        assert_eq!(result.id, "rls_1");
    }

    #[tokio::test]
    async fn cache_miss_backend_hit_caches_value() {
        let (_dir, _, rls_stor) = setup("fb_rls_backend_hit").await;
        let backend_rls = backend_client::Release {
            id: "rls_1".to_string(),
            version: "1.0.0".to_string(),
            git_commit_id: Some("gc_1".to_string()),
            ..Default::default()
        };
        let stub = StubReleaseFetcher::ok(backend_rls);

        let result = rls_svc::get(&rls_stor, Some(&stub), "rls_1".to_string())
            .await
            .unwrap();
        assert_eq!(result.id, "rls_1");
        assert_eq!(result.version, "1.0.0");
        assert_eq!(stub.calls(), 1);

        // Second call with PanicReleaseFetcher must succeed (proves cache).
        let result2 = rls_svc::get(&rls_stor, Some(&PanicReleaseFetcher), "rls_1".to_string())
            .await
            .unwrap();
        assert_eq!(result2.id, "rls_1");
    }

    #[tokio::test]
    async fn cache_miss_backend_404_returns_not_found() {
        let (_dir, _, rls_stor) = setup("fb_rls_404").await;
        let err = ServiceErr::HTTPErr(HTTPErr::RequestFailed(RequestFailed {
            request: HttpParams::get("http://test/cache-miss").meta().unwrap(),
            status: reqwest::StatusCode::NOT_FOUND,
            error: None,
            trace: miru_agent::trace!(),
        }));
        let stub = StubReleaseFetcher::err(err);

        let result = rls_svc::get(&rls_stor, Some(&stub), "rls_1".to_string()).await;
        assert!(matches!(
            result,
            Err(ServiceErr::CacheErr(CacheErr::CacheElementNotFound(_)))
        ));
    }

    #[tokio::test]
    async fn cache_miss_backend_500_returns_error() {
        let (_dir, _, rls_stor) = setup("fb_rls_500").await;
        let err = ServiceErr::HTTPErr(HTTPErr::RequestFailed(RequestFailed {
            request: HttpParams::get("http://test/cache-miss").meta().unwrap(),
            status: reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            error: None,
            trace: miru_agent::trace!(),
        }));
        let stub = StubReleaseFetcher::err(err);

        let result = rls_svc::get(&rls_stor, Some(&stub), "rls_1".to_string()).await;
        assert!(matches!(
            result,
            Err(ServiceErr::HTTPErr(HTTPErr::RequestFailed(_)))
        ));
    }

    #[tokio::test]
    async fn cache_miss_backend_network_err_returns_error() {
        let (_dir, _, rls_stor) = setup("fb_rls_network").await;
        let err = ServiceErr::HTTPErr(HTTPErr::MockErr(HttpMockErr {
            is_network_conn_err: true,
        }));
        let stub = StubReleaseFetcher::err(err);

        let result = rls_svc::get(&rls_stor, Some(&stub), "rls_1".to_string()).await;
        assert!(matches!(result, Err(ServiceErr::HTTPErr(_))));
    }

    #[tokio::test]
    async fn cache_miss_token_err_returns_not_found() {
        let (_dir, _, rls_stor) = setup("fb_rls_token").await;
        let err = ServiceErr::SyncErr(SyncErr::AuthnErr(AuthnErr::MockError(AuthnMockError {
            is_network_conn_err: false,
            trace: miru_agent::trace!(),
        })));
        let stub = StubReleaseFetcher::err(err);

        let result = rls_svc::get(&rls_stor, Some(&stub), "rls_1".to_string()).await;
        assert!(matches!(
            result,
            Err(ServiceErr::CacheErr(CacheErr::CacheElementNotFound(_)))
        ));
    }

    #[tokio::test]
    async fn cache_miss_non_authn_sync_err_propagates() {
        let (_dir, _, rls_stor) = setup("fb_rls_sync_err").await;
        let err = ServiceErr::SyncErr(SyncErr::MockErr(SyncMockErr {
            is_network_conn_err: false,
        }));
        let stub = StubReleaseFetcher::err(err);

        let result = rls_svc::get(&rls_stor, Some(&stub), "rls_1".to_string()).await;
        assert!(matches!(
            result,
            Err(ServiceErr::SyncErr(SyncErr::MockErr(_)))
        ));
    }

    #[tokio::test]
    async fn cache_miss_no_backend_returns_not_found() {
        let (_dir, _, rls_stor) = setup("fb_rls_no_backend").await;

        let result =
            rls_svc::get(&rls_stor, None::<&StubReleaseFetcher>, "rls_1".to_string()).await;
        assert!(matches!(
            result,
            Err(ServiceErr::CacheErr(CacheErr::CacheElementNotFound(_)))
        ));
    }
}
