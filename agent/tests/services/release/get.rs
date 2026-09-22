// internal crates
use crate::mocks::backend::{PanicBackend, StubBackend};
use crate::test_utils::filesys::dirs as test_dirs;
use backend_api::models as backend_client;
use miru_agent::authn::errors::{AuthnErr, MockError as AuthnMockError};
use miru_agent::disk::{FileRules, Releases};
use miru_agent::filesys::Overwrite;
use miru_agent::http::errors::{HTTPErr, MockErr as HttpMockErr, RequestFailed};
use miru_agent::http::request::Params as HttpParams;
use miru_agent::models::{FileRule, Release};
use miru_agent::services::errors::FileRulesNotExpandedErr;
use miru_agent::services::release as rls_svc;
use miru_agent::services::ServiceErr;
use miru_agent::sync::errors::MockErr as SyncMockErr;
use miru_agent::sync::SyncErr;

async fn setup(name: &str) -> (test_dirs::TempDir, Releases, FileRules) {
    let dir = test_dirs::temp(name).unwrap();
    let (rls_stor, _) = Releases::spawn(16, dir.file("releases.json"), 1000)
        .await
        .unwrap();
    let (fr_stor, _) = FileRules::spawn(16, dir.file("file_rules.json"), 1000)
        .await
        .unwrap();
    (dir, rls_stor, fr_stor)
}

pub mod get_release {
    use super::*;

    #[tokio::test]
    async fn returns_release_by_id() {
        let (_dir, rls_stor, fr_stor) = setup("get_rls_by_id").await;
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

        let result = rls_svc::get(&rls_stor, &fr_stor, &PanicBackend, "rls_1".to_string())
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
        let (_dir, rls_stor, fr_stor) = setup("fb_rls_cache_hit").await;
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

        let result = rls_svc::get(&rls_stor, &fr_stor, &PanicBackend, "rls_1".to_string())
            .await
            .unwrap();
        assert_eq!(result.id, "rls_1");
    }

    #[tokio::test]
    async fn cache_miss_backend_hit_caches_value() {
        let (_dir, rls_stor, fr_stor) = setup("fb_rls_backend_hit").await;
        let backend_rls = backend_client::Release {
            id: "rls_1".to_string(),
            version: "1.0.0".to_string(),
            git_commit_id: Some("gc_1".to_string()),
            file_rules: Some(vec![]),
            ..Default::default()
        };
        let stub = StubBackend::new().with_release(Ok(backend_rls));

        let result = rls_svc::get(&rls_stor, &fr_stor, &stub, "rls_1".to_string())
            .await
            .unwrap();
        assert_eq!(result.id, "rls_1");
        assert_eq!(result.version, "1.0.0");
        assert_eq!(stub.release_calls(), 1);

        // Second call with PanicBackend must succeed (proves cache).
        let result2 = rls_svc::get(&rls_stor, &fr_stor, &PanicBackend, "rls_1".to_string())
            .await
            .unwrap();
        assert_eq!(result2.id, "rls_1");
    }

    #[tokio::test]
    async fn cache_miss_backend_release_with_file_rules_links_ids() {
        let (_dir, rls_stor, fr_stor) = setup("fb_rls_file_rules").await;
        let backend_rls = backend_client::Release {
            id: "rls_1".to_string(),
            version: "1.0.0".to_string(),
            file_rules: Some(vec![
                backend_client::BaseFileRule {
                    id: "file_rule_1".to_string(),
                    ..Default::default()
                },
                backend_client::BaseFileRule {
                    id: "file_rule_2".to_string(),
                    ..Default::default()
                },
            ]),
            ..Default::default()
        };
        let stub = StubBackend::new().with_release(Ok(backend_rls));

        let result = rls_svc::get(&rls_stor, &fr_stor, &stub, "rls_1".to_string())
            .await
            .unwrap();
        assert_eq!(
            result.file_rule_ids,
            vec!["file_rule_1".to_string(), "file_rule_2".to_string()]
        );
    }

    #[tokio::test]
    async fn cache_miss_backend_404_propagates_http_err() {
        let (_dir, rls_stor, fr_stor) = setup("fb_rls_404").await;
        let err = ServiceErr::HTTPErr(HTTPErr::RequestFailed(RequestFailed {
            request: HttpParams::get("http://test/cache-miss").meta().unwrap(),
            status: reqwest::StatusCode::NOT_FOUND,
            error: None,
            trace: miru_agent::trace!(),
        }));
        let stub = StubBackend::new().with_release(Err(err));

        let result = rls_svc::get(&rls_stor, &fr_stor, &stub, "rls_1".to_string()).await;
        assert!(matches!(
            result,
            Err(ServiceErr::HTTPErr(HTTPErr::RequestFailed(_)))
        ));
    }

    #[tokio::test]
    async fn cache_miss_backend_500_returns_error() {
        let (_dir, rls_stor, fr_stor) = setup("fb_rls_500").await;
        let err = ServiceErr::HTTPErr(HTTPErr::RequestFailed(RequestFailed {
            request: HttpParams::get("http://test/cache-miss").meta().unwrap(),
            status: reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            error: None,
            trace: miru_agent::trace!(),
        }));
        let stub = StubBackend::new().with_release(Err(err));

        let result = rls_svc::get(&rls_stor, &fr_stor, &stub, "rls_1".to_string()).await;
        assert!(matches!(
            result,
            Err(ServiceErr::HTTPErr(HTTPErr::RequestFailed(_)))
        ));
    }

    #[tokio::test]
    async fn cache_miss_backend_network_err_returns_error() {
        let (_dir, rls_stor, fr_stor) = setup("fb_rls_network").await;
        let err = ServiceErr::HTTPErr(HTTPErr::MockErr(HttpMockErr {
            is_network_conn_err: true,
        }));
        let stub = StubBackend::new().with_release(Err(err));

        let result = rls_svc::get(&rls_stor, &fr_stor, &stub, "rls_1".to_string()).await;
        assert!(matches!(result, Err(ServiceErr::HTTPErr(_))));
    }

    #[tokio::test]
    async fn cache_miss_token_err_propagates_authn_err() {
        let (_dir, rls_stor, fr_stor) = setup("fb_rls_token").await;
        let err = ServiceErr::SyncErr(SyncErr::AuthnErr(AuthnErr::MockError(AuthnMockError {
            is_network_conn_err: false,
            trace: miru_agent::trace!(),
        })));
        let stub = StubBackend::new().with_release(Err(err));

        let result = rls_svc::get(&rls_stor, &fr_stor, &stub, "rls_1".to_string()).await;
        assert!(matches!(
            result,
            Err(ServiceErr::SyncErr(SyncErr::AuthnErr(_)))
        ));
    }

    #[tokio::test]
    async fn cache_miss_non_authn_sync_err_propagates() {
        let (_dir, rls_stor, fr_stor) = setup("fb_rls_sync_err").await;
        let err = ServiceErr::SyncErr(SyncErr::MockErr(SyncMockErr {
            is_network_conn_err: false,
        }));
        let stub = StubBackend::new().with_release(Err(err));

        let result = rls_svc::get(&rls_stor, &fr_stor, &stub, "rls_1".to_string()).await;
        assert!(matches!(
            result,
            Err(ServiceErr::SyncErr(SyncErr::MockErr(_)))
        ));
    }

    #[tokio::test]
    async fn cache_miss_backend_missing_file_rules_errors_and_does_not_cache() {
        let (_dir, rls_stor, fr_stor) = setup("fb_rls_missing_file_rules").await;
        let backend_rls = backend_client::Release {
            id: "rls_1".to_string(),
            version: "1.0.0".to_string(),
            file_rules: None,
            ..Default::default()
        };
        let stub = StubBackend::new().with_release(Ok(backend_rls));

        let result = rls_svc::get(&rls_stor, &fr_stor, &stub, "rls_1".to_string()).await;
        assert!(matches!(
            result,
            Err(ServiceErr::FileRulesNotExpanded(
                FileRulesNotExpandedErr { .. }
            ))
        ));
        assert_eq!(stub.release_calls(), 1);

        // The error must not have been cached: a subsequent get still hits the
        // backend (proven by a fresh stub being invoked exactly once).
        let backend_rls = backend_client::Release {
            id: "rls_1".to_string(),
            version: "1.0.0".to_string(),
            file_rules: Some(vec![]),
            ..Default::default()
        };
        let stub2 = StubBackend::new().with_release(Ok(backend_rls));
        let result2 = rls_svc::get(&rls_stor, &fr_stor, &stub2, "rls_1".to_string())
            .await
            .unwrap();
        assert_eq!(result2.id, "rls_1");
        assert_eq!(stub2.release_calls(), 1);
    }

    fn backend_upload_rule(id: &str) -> backend_client::BaseFileRule {
        backend_client::BaseFileRule {
            id: id.to_string(),
            name: "logs".to_string(),
            upload: Some(Box::new(backend_client::FileRuleUpload {
                upload_collection_id: "uc_1".to_string(),
                upload_collection_name: "logs".to_string(),
                bucket_id: "bkt_1".to_string(),
                bucket_name: "fleet-logs".to_string(),
                path: "robots/".to_string(),
            })),
            retention: Some(Box::new(backend_client::FileRuleRetention {
                require_upload: Some(true),
                ttl_secs: 3600,
            })),
            created_at: "2025-06-15T12:00:00Z".to_string(),
            updated_at: "2025-06-15T12:00:00Z".to_string(),
            ..Default::default()
        }
    }

    fn backend_retention_rule(id: &str) -> backend_client::BaseFileRule {
        backend_client::BaseFileRule {
            id: id.to_string(),
            name: "tmp".to_string(),
            retention: Some(Box::new(backend_client::FileRuleRetention {
                require_upload: None,
                ttl_secs: 60,
            })),
            created_at: "2025-06-15T12:00:00Z".to_string(),
            updated_at: "2025-06-15T12:00:00Z".to_string(),
            ..Default::default()
        }
    }

    fn backend_release(rules: Vec<backend_client::BaseFileRule>) -> backend_client::Release {
        backend_client::Release {
            id: "rls_1".to_string(),
            version: "1.0.0".to_string(),
            file_rules: Some(rules),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn cache_miss_caches_file_rule_bodies() {
        let (_dir, rls_stor, fr_stor) = setup("fb_rls_caches_rule_bodies").await;
        let upload_rule = backend_upload_rule("file_rule_1");
        let retention_rule = backend_retention_rule("file_rule_2");
        let backend_rls = backend_release(vec![upload_rule.clone(), retention_rule.clone()]);
        let stub = StubBackend::new().with_release(Ok(backend_rls));

        rls_svc::get(&rls_stor, &fr_stor, &stub, "rls_1".to_string())
            .await
            .unwrap();

        let cached_1 = fr_stor.read("file_rule_1".to_string()).await.unwrap();
        assert_eq!(cached_1, FileRule::from(upload_rule));
        let cached_2 = fr_stor.read("file_rule_2".to_string()).await.unwrap();
        assert_eq!(cached_2, FileRule::from(retention_rule));

        // The release is cached now that its rule bodies are.
        let result = rls_svc::get(&rls_stor, &fr_stor, &PanicBackend, "rls_1".to_string())
            .await
            .unwrap();
        assert_eq!(
            result.file_rule_ids,
            vec!["file_rule_1".to_string(), "file_rule_2".to_string()]
        );
    }

    #[tokio::test]
    async fn cache_miss_keeps_existing_file_rule_body() {
        let (_dir, rls_stor, fr_stor) = setup("fb_rls_keeps_rule_body").await;
        let seeded = FileRule {
            id: "file_rule_1".to_string(),
            name: "seeded".to_string(),
            ..Default::default()
        };
        fr_stor
            .write_if_absent("file_rule_1".to_string(), seeded.clone(), |_, _| false)
            .await
            .unwrap();
        let backend_rls = backend_release(vec![backend_upload_rule("file_rule_1")]);
        let stub = StubBackend::new().with_release(Ok(backend_rls));

        rls_svc::get(&rls_stor, &fr_stor, &stub, "rls_1".to_string())
            .await
            .unwrap();

        let cached = fr_stor.read("file_rule_1".to_string()).await.unwrap();
        assert_eq!(cached, seeded);
    }

    #[tokio::test]
    async fn cache_miss_rule_cache_failure_does_not_cache_release() {
        let (_dir, rls_stor, fr_stor) = setup("fb_rls_rule_cache_failure").await;
        fr_stor.shutdown().await.unwrap();
        let backend_rls = backend_release(vec![backend_upload_rule("file_rule_1")]);
        let stub = StubBackend::new().with_release(Ok(backend_rls));

        let result = rls_svc::get(&rls_stor, &fr_stor, &stub, "rls_1".to_string())
            .await
            .unwrap();
        assert_eq!(result.file_rule_ids, vec!["file_rule_1".to_string()]);

        let cached = rls_stor.read_optional("rls_1".to_string()).await.unwrap();
        assert!(cached.is_none());
    }
}
