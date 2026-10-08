// internal crates
use crate::mocks::backend::{PanicBackend, StubBackend};
use crate::test_utils::filesys::dirs as test_dirs;
use backend_api::models as backend_client;
use miru_agent::authn::errors::{AuthnErr, MockError as AuthnMockError};
use miru_agent::disk::FileRules;
use miru_agent::http::errors::{HTTPErr, MockErr as HttpMockErr, RequestFailed};
use miru_agent::http::request::Params as HttpParams;
use miru_agent::models::FileRule;
use miru_agent::services::file_rule as file_rule_svc;
use miru_agent::services::ServiceErr;
use miru_agent::sync::errors::MockErr as SyncMockErr;
use miru_agent::sync::SyncErr;

async fn setup(name: &str) -> (test_dirs::TempDir, FileRules) {
    let dir = test_dirs::temp(name).unwrap();
    let (stor, _) = FileRules::spawn(16, dir.file("file_rules.json"), 1000)
        .await
        .unwrap();
    (dir, stor)
}

fn backend_value(id: &str) -> backend_client::BaseFileRule {
    backend_client::BaseFileRule {
        id: id.to_string(),
        name: "logs".to_string(),
        digest: "digest_1".to_string(),
        source: Box::new(backend_client::FileRuleSource {
            glob: "/var/log/app/*.log".to_string(),
            stability_window_secs: 30,
        }),
        ..Default::default()
    }
}

fn request_failed(status: reqwest::StatusCode) -> ServiceErr {
    ServiceErr::HTTPErr(HTTPErr::RequestFailed(RequestFailed {
        request: HttpParams::get("http://test/cache-miss").meta().unwrap(),
        status,
        error: None,
        trace: miru_agent::trace!(),
    }))
}

type ErrCase = (&'static str, ServiceErr, fn(&ServiceErr) -> bool);

fn backend_errors() -> Vec<ErrCase> {
    vec![
        (
            "404",
            request_failed(reqwest::StatusCode::NOT_FOUND),
            |e| matches!(e, ServiceErr::HTTPErr(HTTPErr::RequestFailed(rf)) if rf.status == 404),
        ),
        (
            "500",
            request_failed(reqwest::StatusCode::INTERNAL_SERVER_ERROR),
            |e| matches!(e, ServiceErr::HTTPErr(HTTPErr::RequestFailed(rf)) if rf.status == 500),
        ),
        (
            "network",
            ServiceErr::HTTPErr(HTTPErr::MockErr(HttpMockErr {
                is_network_conn_err: true,
            })),
            |e| matches!(e, ServiceErr::HTTPErr(HTTPErr::MockErr(_))),
        ),
        (
            "authn",
            ServiceErr::SyncErr(SyncErr::AuthnErr(AuthnErr::MockError(AuthnMockError {
                is_network_conn_err: false,
                trace: miru_agent::trace!(),
            }))),
            |e| matches!(e, ServiceErr::SyncErr(SyncErr::AuthnErr(_))),
        ),
        (
            "sync",
            ServiceErr::SyncErr(SyncErr::MockErr(SyncMockErr {
                is_network_conn_err: false,
            })),
            |e| matches!(e, ServiceErr::SyncErr(SyncErr::MockErr(_))),
        ),
    ]
}

pub mod get_file_rule {
    use super::*;

    #[tokio::test]
    async fn cache_hit_no_backend_call() {
        let (_dir, stor) = setup("fr_cache_hit").await;
        let cached = FileRule::from(backend_value("fr_1"));
        stor.write_if_absent("fr_1".to_string(), cached.clone(), |_, _| false)
            .await
            .unwrap();

        let result = file_rule_svc::get(&stor, &PanicBackend, "fr_1".to_string())
            .await
            .unwrap();
        assert_eq!(result, cached);
    }

    #[tokio::test]
    async fn cache_miss_fetches_and_caches() {
        let (_dir, stor) = setup("fr_cache_miss").await;
        let stub = StubBackend::new().with_file_rule(Ok(backend_value("fr_1")));

        let result = file_rule_svc::get(&stor, &stub, "fr_1".to_string())
            .await
            .unwrap();
        assert_eq!(result, FileRule::from(backend_value("fr_1")));
        assert_eq!(stub.file_rule_calls(), 1);

        // the stub has no second response, so a second call must be served from the cache
        let cached = file_rule_svc::get(&stor, &stub, "fr_1".to_string())
            .await
            .unwrap();
        assert_eq!(cached, result);
        assert_eq!(stub.file_rule_calls(), 1);
    }

    #[tokio::test]
    async fn backend_errors_propagate_and_cache_nothing() {
        let (_dir, stor) = setup("fr_backend_errors").await;
        for (name, err, is_expected) in backend_errors() {
            let stub = StubBackend::new().with_file_rule(Err(err));

            let result = file_rule_svc::get(&stor, &stub, "fr_1".to_string()).await;

            let err = result.expect_err(name);
            assert!(is_expected(&err), "{name}: {err:?}");
            assert!(stor
                .read_optional("fr_1".to_string())
                .await
                .unwrap()
                .is_none());
        }
    }

    #[tokio::test]
    async fn cache_read_failure_returns_error_without_backend_call() {
        let (_dir, stor) = setup("fr_cache_read_failure").await;
        stor.shutdown().await.unwrap();
        let stub = StubBackend::new();

        let result = file_rule_svc::get(&stor, &stub, "fr_1".to_string()).await;
        assert!(matches!(result, Err(ServiceErr::CacheErr(_))));
        assert_eq!(stub.file_rule_calls(), 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cache_write_failure_still_returns_value() {
        use miru_agent::filesys::dirs;
        use std::os::unix::fs::PermissionsExt;

        // root ignores directory permissions, so the write cannot be made to fail
        if nix::unistd::geteuid().is_root() {
            return;
        }
        let (dir, stor) = setup("fr_cache_write_failure").await;
        // the cache writes atomically through a temp file in its directory
        let readonly = std::fs::Permissions::from_mode(0o555);
        dirs::set_permissions(dir.dir(), readonly).await.unwrap();
        let stub = StubBackend::new().with_file_rule(Ok(backend_value("fr_1")));

        let result = file_rule_svc::get(&stor, &stub, "fr_1".to_string()).await;

        let readwrite = std::fs::Permissions::from_mode(0o755);
        dirs::set_permissions(dir.dir(), readwrite).await.unwrap();
        assert_eq!(result.unwrap(), FileRule::from(backend_value("fr_1")));
        assert!(stor
            .read_optional("fr_1".to_string())
            .await
            .unwrap()
            .is_none());
    }
}
