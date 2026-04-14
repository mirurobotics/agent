// internal crates
use crate::services::backend_stub::{PanicBackend, StubBackend};
use backend_api::models as backend_client;
use miru_agent::authn::errors::{AuthnErr, MockError as AuthnMockError};
use miru_agent::filesys::{self, Overwrite};
use miru_agent::http::errors::{HTTPErr, MockErr as HttpMockErr, RequestFailed};
use miru_agent::http::request::Params as HttpParams;
use miru_agent::models::{Deployment, DplActivity, DplErrStatus, DplTarget};
use miru_agent::services::deployment as dpl_svc;
use miru_agent::services::ServiceErr;
use miru_agent::storage::Deployments;
use miru_agent::sync::errors::MockErr as SyncMockErr;
use miru_agent::sync::SyncErr;

// external crates
use chrono::{DateTime, Utc};

async fn setup(name: &str) -> (filesys::Dir, Deployments) {
    let dir = filesys::Dir::create_temp_dir(name).await.unwrap();
    let (dpl_stor, _) = Deployments::spawn(16, dir.file("deployments.json"), 1000)
        .await
        .unwrap();
    (dir, dpl_stor)
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

// NOTE: The cache write error branch in cache_deployment is not reachable through
// the public get() API in tests. Inducing a write failure (e.g. via shutdown)
// also breaks the read_optional() call that precedes it.
// The branch is a defensive log-and-continue pattern.

pub mod get_deployment {
    use super::*;

    #[tokio::test]
    async fn returns_deployment_by_id() {
        let (_dir, dpl_stor) = setup("get_dpl_by_id").await;
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

        let result = dpl_svc::get(&dpl_stor, &PanicBackend, "dpl_1".to_string())
            .await
            .unwrap();
        assert_eq!(result.id, "dpl_1");
        assert_eq!(result.activity_status, DplActivity::Deployed);
    }
}

pub mod get_deployment_fallback {
    use super::*;

    #[tokio::test]
    async fn cache_hit_no_backend_call() {
        let (_dir, dpl_stor) = setup("fb_dpl_cache_hit").await;
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

        let result = dpl_svc::get(&dpl_stor, &PanicBackend, "dpl_1".to_string())
            .await
            .unwrap();
        assert_eq!(result.id, "dpl_1");
    }

    #[tokio::test]
    async fn cache_miss_backend_hit_caches_value() {
        let (_dir, dpl_stor) = setup("fb_dpl_backend_hit").await;
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
        let stub = StubBackend::new().with_deployment(Ok(backend_dpl));

        let result = dpl_svc::get(&dpl_stor, &stub, "dpl_1".to_string())
            .await
            .unwrap();
        assert_eq!(result.id, "dpl_1");
        assert_eq!(result.device_id, "dvc_1");
        assert_eq!(result.release_id, "rls_1");
        assert_eq!(result.config_instance_ids, vec!["cfg_1".to_string()]);
        assert_eq!(stub.deployment_calls(), 1);

        // Second call with PanicBackend must succeed (proves cache).
        let result2 = dpl_svc::get(&dpl_stor, &PanicBackend, "dpl_1".to_string())
            .await
            .unwrap();
        assert_eq!(result2.id, "dpl_1");
    }

    #[tokio::test]
    async fn cache_miss_backend_404_propagates_http_err() {
        let (_dir, dpl_stor) = setup("fb_dpl_404").await;
        let err = ServiceErr::HTTPErr(HTTPErr::RequestFailed(RequestFailed {
            request: HttpParams::get("http://test/cache-miss").meta().unwrap(),
            status: reqwest::StatusCode::NOT_FOUND,
            error: None,
            trace: miru_agent::trace!(),
        }));
        let stub = StubBackend::new().with_deployment(Err(err));

        let result = dpl_svc::get(&dpl_stor, &stub, "dpl_1".to_string()).await;
        assert!(matches!(
            result,
            Err(ServiceErr::HTTPErr(HTTPErr::RequestFailed(_)))
        ));
    }

    #[tokio::test]
    async fn cache_miss_backend_500_returns_error() {
        let (_dir, dpl_stor) = setup("fb_dpl_500").await;
        let err = ServiceErr::HTTPErr(HTTPErr::RequestFailed(RequestFailed {
            request: HttpParams::get("http://test/cache-miss").meta().unwrap(),
            status: reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            error: None,
            trace: miru_agent::trace!(),
        }));
        let stub = StubBackend::new().with_deployment(Err(err));

        let result = dpl_svc::get(&dpl_stor, &stub, "dpl_1".to_string()).await;
        assert!(matches!(
            result,
            Err(ServiceErr::HTTPErr(HTTPErr::RequestFailed(_)))
        ));
    }

    #[tokio::test]
    async fn cache_miss_backend_network_err_returns_error() {
        let (_dir, dpl_stor) = setup("fb_dpl_network").await;
        let err = ServiceErr::HTTPErr(HTTPErr::MockErr(HttpMockErr {
            is_network_conn_err: true,
        }));
        let stub = StubBackend::new().with_deployment(Err(err));

        let result = dpl_svc::get(&dpl_stor, &stub, "dpl_1".to_string()).await;
        assert!(matches!(result, Err(ServiceErr::HTTPErr(_))));
    }

    #[tokio::test]
    async fn cache_miss_token_err_propagates_authn_err() {
        let (_dir, dpl_stor) = setup("fb_dpl_token").await;
        let err = ServiceErr::SyncErr(SyncErr::AuthnErr(AuthnErr::MockError(AuthnMockError {
            is_network_conn_err: false,
            trace: miru_agent::trace!(),
        })));
        let stub = StubBackend::new().with_deployment(Err(err));

        let result = dpl_svc::get(&dpl_stor, &stub, "dpl_1".to_string()).await;
        assert!(matches!(
            result,
            Err(ServiceErr::SyncErr(SyncErr::AuthnErr(_)))
        ));
    }

    #[tokio::test]
    async fn cache_miss_non_authn_sync_err_propagates() {
        let (_dir, dpl_stor) = setup("fb_dpl_sync_err").await;
        let err = ServiceErr::SyncErr(SyncErr::MockErr(SyncMockErr {
            is_network_conn_err: false,
        }));
        let stub = StubBackend::new().with_deployment(Err(err));

        let result = dpl_svc::get(&dpl_stor, &stub, "dpl_1".to_string()).await;
        assert!(matches!(
            result,
            Err(ServiceErr::SyncErr(SyncErr::MockErr(_)))
        ));
    }

    #[tokio::test]
    async fn cache_miss_backend_missing_config_instances_returns_sync_err() {
        let (_dir, dpl_stor) = setup("fb_dpl_missing_cfg_insts").await;
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
        let stub = StubBackend::new().with_deployment(Ok(backend_dpl));

        let result = dpl_svc::get(&dpl_stor, &stub, "dpl_1".to_string()).await;
        assert!(matches!(
            result,
            Err(ServiceErr::SyncErr(SyncErr::CfgInstsNotExpanded(_)))
        ));
    }
}

pub mod dirty_flag_predicate {
    use super::*;

    /// Tests the dirty-flag predicate used by `cache_deployment`:
    ///   `|old, _| old.is_some_and(|e| e.is_dirty)`
    ///
    /// This predicate is only reachable via `get()` during a concurrent
    /// write race (cache miss then another caller caches first), so we
    /// validate the predicate logic directly through the storage layer.
    #[tokio::test]
    async fn cache_deployment_preserves_dirty_flag_on_overwrite() {
        let (_dir, dpl_stor) = setup("dirty_flag_predicate").await;
        let dpl = make_deployment("dpl_1", DplActivity::Deployed);

        // 1. Seed a dirty entry by writing with a closure that always returns true.
        dpl_stor
            .write(
                "dpl_1".to_string(),
                dpl.clone(),
                |_, _| true,
                Overwrite::Allow,
            )
            .await
            .unwrap();

        // Verify the entry is dirty.
        let entry = dpl_stor
            .read_entry_optional("dpl_1".to_string())
            .await
            .unwrap()
            .expect("entry should exist");
        assert!(entry.is_dirty, "seeded entry should be dirty");

        // 2. Overwrite using the production predicate.
        //    Since old entry is dirty, the new entry should also be dirty.
        let dpl_updated = make_deployment("dpl_1", DplActivity::Deployed);
        dpl_stor
            .write(
                "dpl_1".to_string(),
                dpl_updated,
                |old, _| old.is_some_and(|e| e.is_dirty),
                Overwrite::Allow,
            )
            .await
            .unwrap();

        let entry = dpl_stor
            .read_entry_optional("dpl_1".to_string())
            .await
            .unwrap()
            .expect("entry should exist after overwrite");
        assert!(
            entry.is_dirty,
            "dirty flag should be preserved when old entry was dirty"
        );

        // 3. Test the inverse: write a non-dirty entry, then overwrite with
        //    the production predicate. The dirty flag should remain false.
        let dpl_clean = make_deployment("dpl_2", DplActivity::Deployed);
        dpl_stor
            .write(
                "dpl_2".to_string(),
                dpl_clean.clone(),
                |_, _| false,
                Overwrite::Allow,
            )
            .await
            .unwrap();

        let entry = dpl_stor
            .read_entry_optional("dpl_2".to_string())
            .await
            .unwrap()
            .expect("clean entry should exist");
        assert!(!entry.is_dirty, "seeded entry should not be dirty");

        // Overwrite with production predicate — old is not dirty, so new should not be either.
        let dpl_clean_updated = make_deployment("dpl_2", DplActivity::Deployed);
        dpl_stor
            .write(
                "dpl_2".to_string(),
                dpl_clean_updated,
                |old, _| old.is_some_and(|e| e.is_dirty),
                Overwrite::Allow,
            )
            .await
            .unwrap();

        let entry = dpl_stor
            .read_entry_optional("dpl_2".to_string())
            .await
            .unwrap()
            .expect("clean entry should exist after overwrite");
        assert!(
            !entry.is_dirty,
            "dirty flag should remain false when old entry was not dirty"
        );
    }
}
