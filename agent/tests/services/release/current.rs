// internal crates
use crate::services::backend_stub::{PanicBackend, StubBackend};
use backend_api::models as backend_client;
use miru_agent::authn::errors::{AuthnErr, MockError as AuthnMockError};
use miru_agent::filesys::{self, Overwrite};
use miru_agent::http::errors::{HTTPErr, RequestFailed};
use miru_agent::http::request::Params as HttpParams;
use miru_agent::models::{Deployment, DplActivity, DplErrStatus, DplTarget, Release};
use miru_agent::services::release as rls_svc;
use miru_agent::services::ServiceErr;
use miru_agent::storage::{Deployments, Releases};
use miru_agent::sync::SyncErr;

// external crates
use chrono::{DateTime, Utc};

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

        let result = rls_svc::get_current(&dpl_stor, &rls_stor, &PanicBackend)
            .await
            .unwrap();
        assert_eq!(result.id, "rls_1");
        assert_eq!(result.version, "2.0.0");
    }

    #[tokio::test]
    async fn no_deployed_deployment_returns_error() {
        let (_dir, dpl_stor, rls_stor) = setup("get_cur_rls_no_dpl").await;

        let result = rls_svc::get_current(&dpl_stor, &rls_stor, &PanicBackend).await;
        assert!(matches!(result, Err(ServiceErr::CacheErr(_))));
    }

    #[tokio::test]
    async fn deployed_deployment_with_missing_release_returns_backend_error() {
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

        let err = ServiceErr::HTTPErr(HTTPErr::RequestFailed(RequestFailed {
            request: HttpParams::get("http://test/cache-miss").meta().unwrap(),
            status: reqwest::StatusCode::NOT_FOUND,
            error: None,
            trace: miru_agent::trace!(),
        }));
        let stub = StubBackend::new().with_release(Err(err));

        let result = rls_svc::get_current(&dpl_stor, &rls_stor, &stub).await;
        assert!(matches!(
            result,
            Err(ServiceErr::HTTPErr(HTTPErr::RequestFailed(_)))
        ));
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

        let result = rls_svc::get_current(&dpl_stor, &rls_stor, &PanicBackend).await;
        assert!(matches!(result, Err(ServiceErr::CacheErr(_))));
    }
}

pub mod get_current_release_fallback {
    use super::*;

    fn make_deployed(release_id: &str) -> Deployment {
        Deployment {
            id: "dpl_1".to_string(),
            release_id: release_id.to_string(),
            activity_status: DplActivity::Deployed,
            error_status: DplErrStatus::None,
            target_status: DplTarget::Deployed,
            created_at: DateTime::<Utc>::UNIX_EPOCH,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn release_cached_no_backend_call() {
        let (_dir, dpl_stor, rls_stor) = setup("fb_cur_rls_cached").await;

        let rls = Release {
            id: "rls_1".to_string(),
            version: "3.0.0".to_string(),
            ..Default::default()
        };
        rls_stor
            .write("rls_1".to_string(), rls, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();

        let dpl = make_deployed("rls_1");
        dpl_stor
            .write("dpl_1".to_string(), dpl, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();

        let result = rls_svc::get_current(&dpl_stor, &rls_stor, &PanicBackend)
            .await
            .unwrap();
        assert_eq!(result.id, "rls_1");
        assert_eq!(result.version, "3.0.0");
    }

    #[tokio::test]
    async fn release_not_cached_backend_returns_release() {
        let (_dir, dpl_stor, rls_stor) = setup("fb_cur_rls_backend_hit").await;

        let dpl = make_deployed("rls_1");
        dpl_stor
            .write("dpl_1".to_string(), dpl, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();

        let backend_rls = backend_client::Release {
            id: "rls_1".to_string(),
            version: "4.0.0".to_string(),
            git_commit_id: Some("gc_1".to_string()),
            ..Default::default()
        };
        let stub = StubBackend::new().with_release(Ok(backend_rls));

        let result = rls_svc::get_current(&dpl_stor, &rls_stor, &stub)
            .await
            .unwrap();
        assert_eq!(result.id, "rls_1");
        assert_eq!(result.version, "4.0.0");
        assert_eq!(stub.release_calls(), 1);
    }

    #[tokio::test]
    async fn release_not_cached_backend_404_returns_error() {
        let (_dir, dpl_stor, rls_stor) = setup("fb_cur_rls_404").await;

        let dpl = make_deployed("rls_1");
        dpl_stor
            .write("dpl_1".to_string(), dpl, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();

        let err = ServiceErr::HTTPErr(HTTPErr::RequestFailed(RequestFailed {
            request: HttpParams::get("http://test/cache-miss").meta().unwrap(),
            status: reqwest::StatusCode::NOT_FOUND,
            error: None,
            trace: miru_agent::trace!(),
        }));
        let stub = StubBackend::new().with_release(Err(err));

        let result = rls_svc::get_current(&dpl_stor, &rls_stor, &stub).await;
        assert!(matches!(
            result,
            Err(ServiceErr::HTTPErr(HTTPErr::RequestFailed(_)))
        ));
    }

    #[tokio::test]
    async fn release_not_cached_backend_token_err_returns_error() {
        let (_dir, dpl_stor, rls_stor) = setup("fb_cur_rls_token").await;

        let dpl = make_deployed("rls_1");
        dpl_stor
            .write("dpl_1".to_string(), dpl, |_, _| false, Overwrite::Allow)
            .await
            .unwrap();

        let err = ServiceErr::SyncErr(SyncErr::AuthnErr(AuthnErr::MockError(AuthnMockError {
            is_network_conn_err: false,
            trace: miru_agent::trace!(),
        })));
        let stub = StubBackend::new().with_release(Err(err));

        let result = rls_svc::get_current(&dpl_stor, &rls_stor, &stub).await;
        assert!(matches!(
            result,
            Err(ServiceErr::SyncErr(SyncErr::AuthnErr(_)))
        ));
    }
}
