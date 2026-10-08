// internal crates
use crate::mocks::backend::{PanicBackend, StubBackend};
use crate::test_utils::filesys::dirs as test_dirs;
use backend_api::models as backend_client;
use miru_agent::disk::FileRules;
use miru_agent::http::errors::{HTTPErr, MockErr as HttpMockErr, RequestFailed};
use miru_agent::http::request::Params as HttpParams;
use miru_agent::models::{FileRule, FileRuleSource, FileRuleUpload};
use miru_agent::services::file_rule as file_rule_svc;
use miru_agent::services::ServiceErr;

async fn setup(name: &str) -> (test_dirs::TempDir, FileRules) {
    let dir = test_dirs::temp(name).unwrap();
    let (stor, _) = FileRules::spawn(16, dir.file("file_rules.json"), 1000)
        .await
        .unwrap();
    (dir, stor)
}

fn backend_rule(id: &str) -> backend_client::BaseFileRule {
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

pub mod get_file_rule {
    use super::*;

    #[tokio::test]
    async fn cache_hit_no_backend_call() {
        let (_dir, stor) = setup("get_file_rule_cached").await;
        let rule = FileRule {
            id: "fr_1".to_string(),
            name: "logs".to_string(),
            digest: "digest_1".to_string(),
            source: FileRuleSource {
                glob: "/var/log/app/*.log".to_string(),
                stability_window_secs: 30,
            },
            upload: Some(FileRuleUpload {
                upload_collection_id: "uc_1".to_string(),
                upload_collection_name: "logs".to_string(),
                bucket_id: "bkt_1".to_string(),
                bucket_name: "fleet-logs".to_string(),
                path: "robots/".to_string(),
            }),
            ..Default::default()
        };
        stor.write_if_absent("fr_1".to_string(), rule.clone(), |_, _| false)
            .await
            .unwrap();

        let result = file_rule_svc::get(&stor, &PanicBackend, "fr_1".to_string())
            .await
            .unwrap();
        assert_eq!(result, rule);
    }

    #[tokio::test]
    async fn cache_miss_backend_hit_caches_value() {
        let (_dir, stor) = setup("get_file_rule_backend_hit").await;
        let stub = StubBackend::new().with_file_rule(Ok(backend_rule("fr_1")));

        let result = file_rule_svc::get(&stor, &stub, "fr_1".to_string())
            .await
            .unwrap();
        assert_eq!(result, FileRule::from(backend_rule("fr_1")));
        assert_eq!(stub.file_rule_calls(), 1);

        // the stub has no second response, so a second call must be served from the cache
        let cached = file_rule_svc::get(&stor, &stub, "fr_1".to_string())
            .await
            .unwrap();
        assert_eq!(cached, result);
        assert_eq!(stub.file_rule_calls(), 1);
    }

    #[tokio::test]
    async fn cache_miss_backend_404_propagates_http_err() {
        let (_dir, stor) = setup("get_file_rule_404").await;
        let stub =
            StubBackend::new().with_file_rule(Err(request_failed(reqwest::StatusCode::NOT_FOUND)));

        let result = file_rule_svc::get(&stor, &stub, "missing".to_string()).await;
        assert!(matches!(
            result,
            Err(ServiceErr::HTTPErr(HTTPErr::RequestFailed(_)))
        ));
        assert!(stor
            .read_optional("missing".to_string())
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn cache_read_failure_returns_error_without_backend_call() {
        let (_dir, stor) = setup("get_file_rule_cache_read_failure").await;
        stor.shutdown().await.unwrap();
        let stub = StubBackend::new();

        let result = file_rule_svc::get(&stor, &stub, "fr_1".to_string()).await;
        assert!(matches!(result, Err(ServiceErr::CacheErr(_))));
        assert_eq!(stub.file_rule_calls(), 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cache_write_failure_still_returns_rule() {
        use miru_agent::filesys::dirs;
        use std::os::unix::fs::PermissionsExt;

        // root ignores directory permissions, so the write cannot be made to fail
        if nix::unistd::geteuid().is_root() {
            return;
        }
        let (dir, stor) = setup("get_file_rule_cache_write_failure").await;
        // the cache writes atomically through a temp file in its directory
        let readonly = std::fs::Permissions::from_mode(0o555);
        dirs::set_permissions(dir.dir(), readonly).await.unwrap();
        let stub = StubBackend::new().with_file_rule(Ok(backend_rule("fr_1")));

        let result = file_rule_svc::get(&stor, &stub, "fr_1".to_string()).await;

        let readwrite = std::fs::Permissions::from_mode(0o755);
        dirs::set_permissions(dir.dir(), readwrite).await.unwrap();
        assert_eq!(result.unwrap(), FileRule::from(backend_rule("fr_1")));
        assert!(stor
            .read_optional("fr_1".to_string())
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn cache_miss_backend_network_err_returns_error() {
        let (_dir, stor) = setup("get_file_rule_network").await;
        let err = ServiceErr::HTTPErr(HTTPErr::MockErr(HttpMockErr {
            is_network_conn_err: true,
        }));
        let stub = StubBackend::new().with_file_rule(Err(err));

        let result = file_rule_svc::get(&stor, &stub, "fr_1".to_string()).await;
        assert!(matches!(result, Err(ServiceErr::HTTPErr(_))));
    }
}
