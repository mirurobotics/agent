// internal crates
use crate::services::backend_stub::{PanicBackend, StubBackend};
use backend_api::models as backend_client;
use miru_agent::authn::errors::{AuthnErr, MockError as AuthnMockError};
use miru_agent::filesys::{self, Overwrite};
use miru_agent::http::errors::{HTTPErr, MockErr as HttpMockErr, RequestFailed};
use miru_agent::http::request::Params as HttpParams;
use miru_agent::models::Release;
use miru_agent::services::release as rls_svc;
use miru_agent::services::ServiceErr;
use miru_agent::storage::Releases;
use miru_agent::sync::errors::MockErr as SyncMockErr;
use miru_agent::sync::SyncErr;

async fn setup(name: &str) -> (filesys::Dir, Releases) {
    let dir = filesys::Dir::create_temp_dir(name).await.unwrap();
    let (rls_stor, _) = Releases::spawn(16, dir.file("releases.json"), 1000)
        .await
        .unwrap();
    (dir, rls_stor)
}

pub mod get_release {
    use super::*;

    #[tokio::test]
    async fn returns_release_by_id() {
        let (_dir, rls_stor) = setup("get_rls_by_id").await;
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

        let result = rls_svc::get(&rls_stor, &PanicBackend, "rls_1".to_string())
            .await
            .unwrap();
        assert_eq!(result.id, "rls_1");
        assert_eq!(result.version, "1.0.0");
    }
}

pub mod get_release_fallback {
    use super::*;

    #[tokio::test]
    async fn cache_hit_no_backend_call() {
        let (_dir, rls_stor) = setup("fb_rls_cache_hit").await;
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

        let result = rls_svc::get(&rls_stor, &PanicBackend, "rls_1".to_string())
            .await
            .unwrap();
        assert_eq!(result.id, "rls_1");
    }

    #[tokio::test]
    async fn cache_miss_backend_hit_caches_value() {
        let (_dir, rls_stor) = setup("fb_rls_backend_hit").await;
        let backend_rls = backend_client::Release {
            id: "rls_1".to_string(),
            version: "1.0.0".to_string(),
            git_commit_id: Some("gc_1".to_string()),
            ..Default::default()
        };
        let stub = StubBackend::new().with_release(Ok(backend_rls));

        let result = rls_svc::get(&rls_stor, &stub, "rls_1".to_string())
            .await
            .unwrap();
        assert_eq!(result.id, "rls_1");
        assert_eq!(result.version, "1.0.0");
        assert_eq!(stub.release_calls(), 1);

        // Second call with PanicBackend must succeed (proves cache).
        let result2 = rls_svc::get(&rls_stor, &PanicBackend, "rls_1".to_string())
            .await
            .unwrap();
        assert_eq!(result2.id, "rls_1");
    }

    #[tokio::test]
    async fn cache_miss_backend_404_propagates_http_err() {
        let (_dir, rls_stor) = setup("fb_rls_404").await;
        let err = ServiceErr::HTTPErr(HTTPErr::RequestFailed(RequestFailed {
            request: HttpParams::get("http://test/cache-miss").meta().unwrap(),
            status: reqwest::StatusCode::NOT_FOUND,
            error: None,
            trace: miru_agent::trace!(),
        }));
        let stub = StubBackend::new().with_release(Err(err));

        let result = rls_svc::get(&rls_stor, &stub, "rls_1".to_string()).await;
        assert!(matches!(
            result,
            Err(ServiceErr::HTTPErr(HTTPErr::RequestFailed(_)))
        ));
    }

    #[tokio::test]
    async fn cache_miss_backend_500_returns_error() {
        let (_dir, rls_stor) = setup("fb_rls_500").await;
        let err = ServiceErr::HTTPErr(HTTPErr::RequestFailed(RequestFailed {
            request: HttpParams::get("http://test/cache-miss").meta().unwrap(),
            status: reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            error: None,
            trace: miru_agent::trace!(),
        }));
        let stub = StubBackend::new().with_release(Err(err));

        let result = rls_svc::get(&rls_stor, &stub, "rls_1".to_string()).await;
        assert!(matches!(
            result,
            Err(ServiceErr::HTTPErr(HTTPErr::RequestFailed(_)))
        ));
    }

    #[tokio::test]
    async fn cache_miss_backend_network_err_returns_error() {
        let (_dir, rls_stor) = setup("fb_rls_network").await;
        let err = ServiceErr::HTTPErr(HTTPErr::MockErr(HttpMockErr {
            is_network_conn_err: true,
        }));
        let stub = StubBackend::new().with_release(Err(err));

        let result = rls_svc::get(&rls_stor, &stub, "rls_1".to_string()).await;
        assert!(matches!(result, Err(ServiceErr::HTTPErr(_))));
    }

    #[tokio::test]
    async fn cache_miss_token_err_propagates_authn_err() {
        let (_dir, rls_stor) = setup("fb_rls_token").await;
        let err = ServiceErr::SyncErr(SyncErr::AuthnErr(AuthnErr::MockError(AuthnMockError {
            is_network_conn_err: false,
            trace: miru_agent::trace!(),
        })));
        let stub = StubBackend::new().with_release(Err(err));

        let result = rls_svc::get(&rls_stor, &stub, "rls_1".to_string()).await;
        assert!(matches!(
            result,
            Err(ServiceErr::SyncErr(SyncErr::AuthnErr(_)))
        ));
    }

    #[tokio::test]
    async fn cache_miss_non_authn_sync_err_propagates() {
        let (_dir, rls_stor) = setup("fb_rls_sync_err").await;
        let err = ServiceErr::SyncErr(SyncErr::MockErr(SyncMockErr {
            is_network_conn_err: false,
        }));
        let stub = StubBackend::new().with_release(Err(err));

        let result = rls_svc::get(&rls_stor, &stub, "rls_1".to_string()).await;
        assert!(matches!(
            result,
            Err(ServiceErr::SyncErr(SyncErr::MockErr(_)))
        ));
    }
}
